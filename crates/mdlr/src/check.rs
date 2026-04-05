use anyhow::{Result, bail};
use std::collections::HashSet;
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

use crate::cache::CacheStore;
use crate::cli::OutputFormat;
use crate::config;
use crate::extraction::{
    extract_go, extract_py, extract_rust, extract_ts, load_entries_from_dir,
};
use crate::find_project_root;
use crate::json_output::{
    build_bucketed_json, build_complexity_json, build_fan_metrics_json,
    build_file_loc_json, build_struct_json,
};
use crate::metrics_rows::{MetricsBundle, collect_metric_rows};
use crate::timing;
use mdlr_core::{Graph, Unit, build as build_graph};
use mdlr_metrics::{
    BucketedMetrics, ComplexityMetrics, FileLocMetrics, StructMetrics,
    StructuralMetrics, Thresholds,
    compute_with_hub_thresholds as compute_structural,
};

/// Represents what type of filter was specified
enum CheckFilter {
    /// No filter - analyze entire project
    None,
    /// Filter by file path
    File(PathBuf),
    /// Filter by directory path
    Directory(PathBuf),
    /// Filter by symbol ID
    Symbol(String),
    /// Filter by git diff — only files changed on the current branch
    Diff(HashSet<PathBuf>),
}

/// Bundle of all computed metrics for a graph
struct ComputedMetrics {
    graph: Graph,
    structural: StructuralMetrics,
    complexity: ComplexityMetrics,
    struct_metrics: StructMetrics,
    file_loc: FileLocMetrics,
}

/// Context for the check command, bundling common resources
struct CheckContext {
    cwd: std::path::PathBuf,
    store: CacheStore,
    config: config::Config,
    /// Generation ID (unix timestamp) shared across all extractors.
    /// Cache entries with `cached_at < generation_id` are stale.
    generation_id: u64,
}

impl CheckContext {
    fn new(explicit_root: Option<&Path>) -> Result<Self> {
        let cwd = env::current_dir()?;
        let root = find_project_root(&cwd, explicit_root);
        let store = CacheStore::open(&root)?;
        let config = config::load_from_dir(store.root())?;
        let generation_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(CheckContext { cwd, store, config, generation_id })
    }
}

/// Parse target string into a CheckFilter
fn parse_check_filter(target: Option<&str>, cwd: &Path) -> CheckFilter {
    if let Some(target_str) = target {
        let target_path = if Path::new(target_str).is_absolute() {
            Path::new(target_str).to_path_buf()
        } else {
            cwd.join(target_str)
        };

        if target_path.exists() {
            let canonical = target_path.canonicalize().unwrap_or(target_path);
            if canonical.is_file() {
                CheckFilter::File(canonical)
            } else {
                CheckFilter::Directory(canonical)
            }
        } else {
            CheckFilter::Symbol(target_str.to_string())
        }
    } else {
        CheckFilter::None
    }
}

/// Check if a file path passes the filter.
/// When `folder` is set, also requires the file to be inside that directory.
fn passes_path_filter(
    file_path: &Path,
    filter: &CheckFilter,
    folder: Option<&Path>,
) -> bool {
    let passes_mode = match filter {
        CheckFilter::File(filter_path) => file_path == *filter_path,
        CheckFilter::Directory(filter_path) => {
            file_path.starts_with(filter_path)
        }
        CheckFilter::Diff(changed) => {
            file_path.canonicalize().map_or(false, |p| changed.contains(&p))
        }
        CheckFilter::Symbol(_) | CheckFilter::None => true,
    };
    if !passes_mode {
        return false;
    }
    if let Some(folder) = folder {
        file_path.canonicalize().map_or(false, |p| p.starts_with(folder))
    } else {
        true
    }
}

/// Check if the current HEAD is on the base branch (main or master).
fn is_on_base_branch(root: &Path) -> bool {
    let output = process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(root)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let branch = String::from_utf8_lossy(&o.stdout).trim().to_string();
            branch == "main" || branch == "master"
        }
        _ => false,
    }
}

/// Detect the base branch by checking if `main` or `master` exists.
fn detect_base_branch(root: &Path) -> Result<String> {
    for branch in &["main", "master"] {
        let output = process::Command::new("git")
            .args(["rev-parse", "--verify", branch])
            .current_dir(root)
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .status()?;
        if output.success() {
            return Ok(branch.to_string());
        }
    }
    bail!("Could not detect base branch: neither 'main' nor 'master' exists")
}

/// Get staged and unstaged changes relative to HEAD.
/// Used when on main/master to check only the current working-tree modifications.
fn diff_files_head(root: &Path) -> Result<HashSet<PathBuf>> {
    let staged = git_diff_name_only(root, &["--cached"])?;
    let unstaged = git_diff_name_only(root, &[])?;

    let mut changed = HashSet::new();
    for rel in staged.iter().chain(unstaged.iter()) {
        let abs = root.join(rel);
        if let Ok(canonical) = abs.canonicalize() {
            changed.insert(canonical);
        }
    }

    Ok(changed)
}

/// Get the set of files changed on the current branch relative to its base.
/// Includes committed, staged, and unstaged changes (but not untracked files).
fn diff_files(root: &Path) -> Result<HashSet<PathBuf>> {
    let base = detect_base_branch(root)?;

    // Find merge base
    let merge_base_output = process::Command::new("git")
        .args(["merge-base", "HEAD", &base])
        .current_dir(root)
        .output()?;
    if !merge_base_output.status.success() {
        bail!(
            "git merge-base failed — are you on a branch that shares history with '{}'?",
            base
        );
    }
    let merge_base =
        String::from_utf8_lossy(&merge_base_output.stdout).trim().to_string();

    // Committed changes since merge base
    let committed = git_diff_name_only(root, &[&merge_base, "HEAD"])?;
    // Staged changes
    let staged = git_diff_name_only(root, &["--cached"])?;
    // Unstaged changes
    let unstaged = git_diff_name_only(root, &[])?;

    let mut changed = HashSet::new();
    for rel in committed.iter().chain(staged.iter()).chain(unstaged.iter()) {
        let abs = root.join(rel);
        if let Ok(canonical) = abs.canonicalize() {
            changed.insert(canonical);
        }
    }

    Ok(changed)
}

/// Run `git diff --name-only` with the given extra args and return the list of paths.
fn git_diff_name_only(root: &Path, args: &[&str]) -> Result<Vec<String>> {
    let mut cmd = process::Command::new("git");
    cmd.arg("diff").arg("--name-only");
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd.current_dir(root).output()?;
    if !output.status.success() {
        bail!("git diff --name-only failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

/// Compute all metrics from units
#[tracing::instrument(name = "compute_metrics", skip_all)]
fn compute_all_metrics(
    units: Vec<Unit>,
    config: &config::Config,
) -> ComputedMetrics {
    let graph =
        tracing::info_span!("build_graph").in_scope(|| build_graph(units));
    let structural = compute_structural(
        &graph,
        config.hub.min_fan_in,
        config.hub.min_fan_out,
    );
    let complexity = ComplexityMetrics::compute(&graph);
    let struct_metrics = StructMetrics::compute(&graph);
    let file_loc = FileLocMetrics::compute(&graph);

    ComputedMetrics { graph, structural, complexity, struct_metrics, file_loc }
}

/// Extract symbol filter string from CheckFilter
fn get_symbol_filter(filter: &CheckFilter) -> Option<&str> {
    match filter {
        CheckFilter::Symbol(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Format and print text output
fn format_text_output(
    computed: &ComputedMetrics,
    config: &config::Config,
    k: i32,
    pretty: bool,
    filter: &CheckFilter,
    store: &CacheStore,
) -> Result<()> {
    let bundle = MetricsBundle {
        structural: &computed.structural,
        complexity: &computed.complexity,
        struct_metrics: &computed.struct_metrics,
        file_loc: &computed.file_loc,
    };
    let symbol_filter = get_symbol_filter(filter);
    let ignores = store.ignores().load_ignores().unwrap_or_default();
    let rows =
        collect_metric_rows(&bundle, config, k, symbol_filter, &ignores);

    if pretty {
        let mut tw = tabwriter::TabWriter::new(vec![]);
        writeln!(tw, "metric\tsymbol\tvalue\tbucket")?;
        for (metric, symbol, value, bucket) in &rows {
            writeln!(tw, "{}\t{}\t{}\t{}", metric, symbol, value, bucket)?;
        }
        tw.flush()?;
        print!("{}", String::from_utf8_lossy(&tw.into_inner()?));
    } else {
        println!("metric\tsymbol\tvalue\tbucket");
        for (metric, symbol, value, bucket) in &rows {
            println!("{}\t{}\t{}\t{}", metric, symbol, value, bucket);
        }
    }

    let partial_count =
        computed.graph.units.iter().filter(|u| u.partial).count();
    if partial_count > 0 {
        eprintln!(
            "warning: {} unit(s) have partial extraction (compilation errors prevented full analysis)",
            partial_count
        );
    }

    Ok(())
}

/// Format and print JSON output
fn format_json_output(
    computed: &ComputedMetrics,
    config: &config::Config,
    extracted_count: usize,
    filter: &CheckFilter,
) -> Result<()> {
    // When filtering by symbol, output specific metrics for that symbol
    if let CheckFilter::Symbol(symbol_id) = filter {
        let output = build_symbol_json(computed, config, symbol_id);
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let thresholds = Thresholds::default();
    let bucketed =
        BucketedMetrics::from_metrics(&computed.structural, &thresholds);

    let partial_count =
        computed.graph.units.iter().filter(|u| u.partial).count();

    let output = serde_json::json!({
        "files": {
            "extracted": extracted_count,
        },
        "units": computed.graph.units.len(),
        "partial_units": partial_count,
        "edges": computed.graph.edges.len(),
        "metrics": {
            "dag_density": build_bucketed_json(&bucketed.dag_density),
            "fan_in": build_fan_metrics_json(&bucketed.fan_in, &computed.structural.fan_in.distribution),
            "fan_out": build_fan_metrics_json(&bucketed.fan_out, &computed.structural.fan_out.distribution),
            "complexity": build_complexity_json(&computed.complexity),
            "struct": build_struct_json(&computed.struct_metrics),
            "file_loc": build_file_loc_json(&computed.file_loc),
        }
    });
    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

/// Insert a metric entry for a symbol if found in the distribution.
fn insert_symbol_metric(
    metrics: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    distribution: &[(String, usize)],
    thresholds: &config::MetricThresholds,
    symbol_id: &str,
) {
    if let Some((_, value)) = distribution.iter().find(|(n, _)| n == symbol_id)
    {
        let bucket = thresholds.evaluate(*value as f64);
        metrics.insert(
            name.to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }
}

/// Build JSON output for a specific symbol
fn build_symbol_json(
    computed: &ComputedMetrics,
    config: &config::Config,
    symbol_id: &str,
) -> serde_json::Value {
    let mut metrics = serde_json::Map::new();
    let t = &config.thresholds;

    let metric_sources: &[(
        &str,
        &[(String, usize)],
        &config::MetricThresholds,
    )] = &[
        ("fan_in", &computed.structural.fan_in.distribution, &t.fan_in_max),
        ("fan_out", &computed.structural.fan_out.distribution, &t.fan_out_max),
        (
            "function_size",
            &computed.complexity.size.distribution,
            &t.function_size,
        ),
        ("params", &computed.complexity.params.distribution, &t.params),
        (
            "cyclomatic",
            &computed.complexity.cyclomatic.distribution,
            &t.cyclomatic,
        ),
        (
            "cognitive",
            &computed.complexity.cognitive.distribution,
            &t.cognitive,
        ),
        (
            "max_scope",
            &computed.complexity.max_scope.distribution,
            &t.max_scope,
        ),
        (
            "methods_per_struct",
            &computed.struct_metrics.methods_per_struct.distribution,
            &t.methods_per_struct,
        ),
        ("lcom", &computed.struct_metrics.lcom.distribution, &t.lcom),
    ];

    for (name, distribution, thresholds) in metric_sources {
        insert_symbol_metric(
            &mut metrics,
            name,
            distribution,
            thresholds,
            symbol_id,
        );
    }

    let is_partial =
        computed.graph.units.iter().any(|u| u.id == symbol_id && u.partial);

    let mut output = serde_json::json!({
        "symbol": symbol_id,
        "metrics": metrics
    });
    if is_partial {
        output["partial"] = serde_json::json!(true);
    }
    output
}

/// Set up timing instrumentation if requested, returns a printer to call after work is done.
fn setup_timing(enabled: bool) -> Option<timing::TimingPrinter> {
    if !enabled {
        return None;
    }
    let (layer, printer) = timing::TimingLayer::new();
    let subscriber = tracing_subscriber::registry::Registry::default();
    use tracing_subscriber::layer::SubscriberExt;
    let subscriber = subscriber.with(layer);
    tracing::subscriber::set_global_default(subscriber)
        .expect("failed to set tracing subscriber");
    Some(printer)
}

/// Load cache entries and collect units matching the filter.
/// Entries with `cached_at < generation_id` are stale and skipped.
fn load_filtered_units(
    store: &CacheStore,
    filter: &CheckFilter,
    folder: Option<&Path>,
    generation_id: u64,
) -> Result<(Vec<crate::cache::FileCacheEntry>, Vec<Unit>)> {
    let mut all_entries = Vec::new();
    load_entries_from_dir(&store.cache_dir(), &mut all_entries)?;

    let mut entries = Vec::new();
    let mut units = Vec::new();
    for entry in all_entries {
        if entry.cached_at < generation_id {
            continue; // stale entry from a previous extraction
        }
        let file_path = store.root().join(&entry.source_path);
        if passes_path_filter(&file_path, filter, folder) {
            units.extend(entry.units.clone());
        }
        entries.push(entry);
    }

    Ok((entries, units))
}

/// Extract, load, validate, and compute all metrics.
fn extract_and_analyze(
    ctx: &CheckContext,
    filter: &CheckFilter,
    folder: Option<&Path>,
) -> Result<(ComputedMetrics, usize)> {
    extract_rust(&ctx.store, ctx.generation_id)?;
    if let Err(e) = extract_ts(&ctx.store, ctx.generation_id) {
        eprintln!("Warning: TS extraction failed: {e:#}");
    }
    if let Err(e) = extract_go(&ctx.store, ctx.generation_id) {
        eprintln!("Warning: Go extraction failed: {e:#}");
    }
    if let Err(e) = extract_py(&ctx.store, ctx.generation_id) {
        eprintln!("Warning: Python extraction failed: {e:#}");
    }

    let (entries, units) =
        load_filtered_units(&ctx.store, filter, folder, ctx.generation_id)?;

    if let CheckFilter::Symbol(symbol_id) = filter {
        if !units.iter().any(|u| u.id == *symbol_id) {
            bail!(
                "Symbol '{}' not found. Run 'mdlr ls' to see available symbols.",
                symbol_id
            );
        }
    }

    let entry_count = entries.len();
    let computed = compute_all_metrics(units, &ctx.config);
    Ok((computed, entry_count))
}

pub fn handle_check(
    target: Option<&str>,
    k: i32,
    pretty: bool,
    format: OutputFormat,
    timing: bool,
    all: bool,
    filter_dir: Option<&str>,
    explicit_root: Option<&Path>,
) -> Result<()> {
    let printer = setup_timing(timing);
    let ctx = CheckContext::new(explicit_root)?;

    // Resolve --filter directory to a canonical path
    let folder = if let Some(dir) = filter_dir {
        let p = if Path::new(dir).is_absolute() {
            PathBuf::from(dir)
        } else {
            ctx.cwd.join(dir)
        };
        let canonical = p.canonicalize().map_err(|_| {
            anyhow::anyhow!("filter directory '{}' does not exist", dir)
        })?;
        if !canonical.is_dir() {
            bail!("filter path '{}' is not a directory", dir);
        }
        Some(canonical)
    } else {
        None
    };

    let filter = if target.is_some() || all {
        // Explicit target or --all flag: skip diff mode
        parse_check_filter(target, &ctx.cwd)
    } else if is_on_base_branch(ctx.store.root()) {
        // On main/master: only check staged + unstaged changes against HEAD
        let changed = diff_files_head(ctx.store.root())?;
        CheckFilter::Diff(changed)
    } else {
        // On a branch: diff mode by default
        let changed = diff_files(ctx.store.root())?;
        CheckFilter::Diff(changed)
    };

    let (computed, entry_count) =
        extract_and_analyze(&ctx, &filter, folder.as_deref())?;

    let result = match format {
        OutputFormat::Text => format_text_output(
            &computed,
            &ctx.config,
            k,
            pretty,
            &filter,
            &ctx.store,
        ),
        OutputFormat::Json => {
            format_json_output(&computed, &ctx.config, entry_count, &filter)
        }
    };

    if let Some(printer) = printer {
        printer.print();
    }

    result
}
