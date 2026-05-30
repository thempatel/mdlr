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
    extract_go, extract_py, extract_rust, extract_ts, has_python_project,
    has_ts_files, load_entries_from_dir, load_tokens_from_dir,
};
use crate::find_project_root;
use crate::json_output::{
    build_bucketed_json, build_complexity_json, build_fan_metrics_json,
    build_file_loc_json, build_struct_json,
};
use crate::metrics_rows::{MetricsBundle, collect_metric_rows};
use crate::progress::CheckProgress;
use crate::timing;
use mdlr_core::{Graph, Unit, build_with_progress as build_graph};
use mdlr_metrics::{
    BucketedMetrics, ComplexityMetrics, CoverageMetrics, FileLocMetrics,
    LcovData, StructMetrics, StructuralMetrics, Thresholds,
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
    duplication: mdlr_cpd::DuplicationMetrics,
    coverage: Option<CoverageMetrics>,
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

#[tracing::instrument(name = "compute_metrics", skip_all)]
fn compute_all_metrics(
    units: Vec<Unit>,
    all_tokens: &[mdlr_cpd::FileTokens],
    scope_files: Option<&HashSet<PathBuf>>,
    config: &config::Config,
    progress: &CheckProgress,
    cov_files: &[PathBuf],
    repo_root: &Path,
) -> ComputedMetrics {
    let unit_count = units.len() as u64;
    let bar = progress.start_bar("Building graph", unit_count);
    let graph = tracing::info_span!("build_graph")
        .in_scope(|| build_graph(units, |i| bar.set_position(i as u64)));
    bar.finish();

    let total = graph.units.len() as u64;
    let bar = progress.start_bar("Computing metrics", total * 4);
    let structural = compute_structural(
        &graph,
        config.hub.min_fan_in,
        config.hub.min_fan_out,
        |i| bar.set_position(i as u64),
    );
    let complexity = ComplexityMetrics::compute_with_progress(&graph, |i| {
        bar.set_position(total + i as u64)
    });
    let struct_metrics = StructMetrics::compute_with_progress(&graph, |i| {
        bar.set_position(total * 2 + i as u64)
    });
    let file_loc = FileLocMetrics::compute_with_progress(&graph, |i| {
        bar.set_position(total * 3 + i as u64)
    });
    bar.finish();

    // CPD is the expensive pass; skip it entirely when duplication_pct is off.
    let duplication = if config.is_disabled("duplication_pct") {
        mdlr_cpd::DuplicationMetrics::default()
    } else {
        let token_file_count = all_tokens.len() as u64;
        let bar = progress.start_bar("Detecting duplicates", token_file_count);
        let duplication = tracing::info_span!("cpd").in_scope(|| {
            let clones = mdlr_cpd::find_clones_with_progress(
                all_tokens,
                config.cpd.min_tokens,
                |i| bar.set_position(i as u64),
            );
            mdlr_cpd::compute_duplication(&clones, all_tokens, scope_files)
        });
        bar.finish();
        duplication
    };

    // Skip coverage parsing when both coverage metrics are disabled.
    let coverage_disabled =
        config.is_disabled("line_cov") && config.is_disabled("uncov_branches");
    let coverage = if cov_files.is_empty() || coverage_disabled {
        None
    } else {
        let spinner = progress.start_spinner("Loading coverage");
        let mut lcov = LcovData::new();
        let mut load_warnings: Vec<String> = Vec::new();
        for path in cov_files {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                repo_root.join(path)
            };
            if let Err(e) = lcov.parse_and_merge(&resolved, repo_root) {
                load_warnings.push(format!(
                    "skipped --cov {}: {e}",
                    resolved.display()
                ));
            }
        }
        if load_warnings.is_empty() {
            spinner.finish();
        } else {
            spinner.finish_warn("partial");
        }
        for w in &load_warnings {
            progress.warn(w);
        }
        let cov =
            CoverageMetrics::compute(&graph, &lcov, repo_root, scope_files);
        if cov.units_analyzed > 0
            && cov.lcov_files_total > 0
            && cov.lcov_files_matched == 0
        {
            progress.warn(&format!(
                "lcov references {} file(s) but none match any analyzed source — check that SF: paths point at source files mdlr sees (often a sourcemap issue: lcov references built .js while graph holds .ts, or paths are rooted differently than --root)",
                cov.lcov_files_total
            ));
        } else if cov.units_analyzed > 0
            && cov.units_without_data * 2 >= cov.units_analyzed
        {
            progress.warn(&format!(
                "{}/{} analyzed units had no coverage data — is the lcov file stale or incomplete?",
                cov.units_without_data, cov.units_analyzed
            ));
        }
        if !cov.has_branches {
            progress.warn(
                "lcov has no BRDA records — uncov_branches omitted (re-run coverage with branch instrumentation: c8 --all, coverage run --branch, llvm-cov --branch)",
            );
        }
        Some(cov)
    };

    ComputedMetrics {
        graph,
        structural,
        complexity,
        struct_metrics,
        file_loc,
        duplication,
        coverage,
    }
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
        duplication: &computed.duplication,
        coverage: computed.coverage.as_ref(),
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

    let duplication_json = serde_json::json!({
        "max": computed.duplication.max,
        "mean": computed.duplication.mean,
        "p90": computed.duplication.p90,
        "clone_count": computed.duplication.clone_count,
        "distribution": computed.duplication.distribution.iter()
            .map(|(file, pct)| serde_json::json!({"file": file, "duplication_pct": pct}))
            .collect::<Vec<_>>(),
    });

    // Build per-metric, omitting disabled metrics — including inside the
    // composite `complexity`/`struct`/`coverage` objects (dropped entirely if
    // every metric they hold is disabled).
    let enabled = |name: &str| !config.is_disabled(name);
    let prune = |mut obj: serde_json::Value,
                 fields: &[(&str, &str)]|
     -> Option<serde_json::Value> {
        let map = obj.as_object_mut().expect("builder returns an object");
        for (json_key, metric) in fields {
            if config.is_disabled(metric) {
                map.remove(*json_key);
            }
        }
        if map.is_empty() { None } else { Some(obj) }
    };

    let mut metrics_json = serde_json::Map::new();
    if enabled("dag_density") {
        metrics_json.insert(
            "dag_density".into(),
            build_bucketed_json(&bucketed.dag_density),
        );
    }
    if enabled("fan_in") {
        metrics_json.insert(
            "fan_in".into(),
            build_fan_metrics_json(
                &bucketed.fan_in,
                &computed.structural.fan_in.distribution,
            ),
        );
    }
    if enabled("fan_out") {
        metrics_json.insert(
            "fan_out".into(),
            build_fan_metrics_json(
                &bucketed.fan_out,
                &computed.structural.fan_out.distribution,
            ),
        );
    }
    if let Some(complexity) = prune(
        build_complexity_json(&computed.complexity),
        &[
            ("size", "function_size"),
            ("params", "params"),
            ("cyclomatic", "cyclomatic"),
            ("max_scope", "max_scope"),
        ],
    ) {
        metrics_json.insert("complexity".into(), complexity);
    }
    if let Some(struct_json) = prune(
        build_struct_json(&computed.struct_metrics),
        &[("methods_per_struct", "methods_per_struct"), ("lcom", "lcom")],
    ) {
        metrics_json.insert("struct".into(), struct_json);
    }
    if enabled("file_loc") {
        metrics_json.insert(
            "file_loc".into(),
            build_file_loc_json(&computed.file_loc),
        );
    }
    if enabled("duplication_pct") {
        metrics_json.insert("duplication".into(), duplication_json);
    }
    if let Some(cov) = computed.coverage.as_ref() {
        if let Some(coverage) = prune(
            crate::json_output::build_coverage_json(cov),
            &[("line_cov", "line_cov"), ("uncov_branches", "uncov_branches")],
        ) {
            metrics_json.insert("coverage".into(), coverage);
        }
    }
    let metrics_json = serde_json::Value::Object(metrics_json);
    let output = serde_json::json!({
        "files": {
            "extracted": extracted_count,
        },
        "units": computed.graph.units.len(),
        "partial_units": partial_count,
        "edges": computed.graph.edges.len(),
        "metrics": metrics_json,
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
    direction: mdlr_metrics::SortDirection,
) {
    if let Some((_, value)) = distribution.iter().find(|(n, _)| n == symbol_id)
    {
        let bucket = match direction {
            mdlr_metrics::SortDirection::Desc => {
                thresholds.evaluate(*value as f64)
            }
            mdlr_metrics::SortDirection::Asc => {
                thresholds.evaluate_asc(*value as f64)
            }
        };
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

    use mdlr_metrics::SortDirection::{Asc, Desc};
    let mut metric_sources: Vec<(
        &str,
        &[(String, usize)],
        &config::MetricThresholds,
        mdlr_metrics::SortDirection,
    )> = vec![
        (
            "fan_in",
            &computed.structural.fan_in.distribution,
            &t.fan_in_max,
            Desc,
        ),
        (
            "fan_out",
            &computed.structural.fan_out.distribution,
            &t.fan_out_max,
            Desc,
        ),
        (
            "function_size",
            &computed.complexity.size.distribution,
            &t.function_size,
            Desc,
        ),
        ("params", &computed.complexity.params.distribution, &t.params, Desc),
        (
            "cyclomatic",
            &computed.complexity.cyclomatic.distribution,
            &t.cyclomatic,
            Desc,
        ),
        (
            "cognitive",
            &computed.complexity.cognitive.distribution,
            &t.cognitive,
            Desc,
        ),
        (
            "max_scope",
            &computed.complexity.max_scope.distribution,
            &t.max_scope,
            Desc,
        ),
        (
            "methods_per_struct",
            &computed.struct_metrics.methods_per_struct.distribution,
            &t.methods_per_struct,
            Desc,
        ),
        ("lcom", &computed.struct_metrics.lcom.distribution, &t.lcom, Desc),
        (
            "duplication_pct",
            &computed.duplication.distribution,
            &t.duplication_pct,
            Desc,
        ),
    ];
    if let Some(cov) = computed.coverage.as_ref() {
        metric_sources.push((
            "line_cov",
            &cov.line_cov.distribution,
            &t.line_cov,
            Asc,
        ));
        if cov.has_branches {
            metric_sources.push((
                "uncov_branches",
                &cov.uncov_branches.distribution,
                &t.uncov_branches,
                Desc,
            ));
        }
    }

    metric_sources.retain(|(name, ..)| !config.is_disabled(name));

    for (name, distribution, thresholds, direction) in &metric_sources {
        insert_symbol_metric(
            &mut metrics,
            name,
            distribution,
            thresholds,
            symbol_id,
            *direction,
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
/// Also loads all token caches for CPD (which needs project-wide data).
fn load_filtered_units(
    store: &CacheStore,
    filter: &CheckFilter,
    folder: Option<&Path>,
    generation_id: u64,
) -> Result<(
    Vec<crate::cache::FileCacheEntry>,
    Vec<Unit>,
    Vec<mdlr_cpd::FileTokens>,
    Option<HashSet<PathBuf>>,
)> {
    let mut all_entries = Vec::new();
    load_entries_from_dir(&store.cache_dir(), &mut all_entries)?;

    let mut all_tokens = Vec::new();
    load_tokens_from_dir(&store.cache_dir(), &mut all_tokens)?;

    // Filter stale token caches
    all_tokens.retain(|t| t.cached_at >= generation_id);

    let mut entries = Vec::new();
    let mut units = Vec::new();
    let mut scope_files: Option<HashSet<PathBuf>> = match filter {
        CheckFilter::None => None,
        _ => Some(HashSet::new()),
    };

    for entry in all_entries {
        if entry.cached_at < generation_id {
            continue; // stale entry from a previous extraction
        }
        let file_path = store.root().join(&entry.source_path);
        if passes_path_filter(&file_path, filter, folder) {
            units.extend(entry.units.clone());
            if let Some(ref mut scope) = scope_files {
                scope.insert(entry.source_path.clone());
            }
        }
        entries.push(entry);
    }

    Ok((entries, units, all_tokens, scope_files))
}

fn run_extractor(
    name: &str,
    progress: &CheckProgress,
    f: impl FnOnce() -> Result<bool>,
) {
    let spinner = progress.start_spinner(name);
    match f() {
        Ok(true) => spinner.finish(),
        Ok(false) => spinner.finish_warn("partial"),
        Err(e) => {
            spinner.finish_warn("failed");
            eprintln!("Warning: {name} failed: {e:#}");
        }
    }
}

/// Extract, load, validate, and compute all metrics.
fn extract_and_analyze(
    ctx: &CheckContext,
    filter: &CheckFilter,
    folder: Option<&Path>,
    progress: &CheckProgress,
    cov_files: &[PathBuf],
) -> Result<(ComputedMetrics, usize)> {
    let root = ctx.store.root();

    if root.join("Cargo.toml").exists() {
        run_extractor("Extracting Rust", progress, || {
            extract_rust(&ctx.store, ctx.generation_id)
        });
    }
    if has_ts_files(root) {
        run_extractor("Extracting TypeScript", progress, || {
            extract_ts(&ctx.store, ctx.generation_id)
        });
    }
    if root.join("go.mod").exists() {
        run_extractor("Extracting Go", progress, || {
            extract_go(&ctx.store, ctx.generation_id)
        });
    }
    if has_python_project(root) {
        run_extractor("Extracting Python", progress, || {
            extract_py(&ctx.store, ctx.generation_id)
        });
    }

    let spinner = progress.start_spinner("Loading cache");
    let (entries, units, all_tokens, scope_files) =
        load_filtered_units(&ctx.store, filter, folder, ctx.generation_id)?;
    spinner.finish();

    if let CheckFilter::Symbol(symbol_id) = filter {
        if !units.iter().any(|u| u.id == *symbol_id) {
            bail!(
                "Symbol '{}' not found. Run 'mdlr ls' to see available symbols.",
                symbol_id
            );
        }
    }

    let entry_count = entries.len();
    let computed = compute_all_metrics(
        units,
        &all_tokens,
        scope_files.as_ref(),
        &ctx.config,
        progress,
        cov_files,
        ctx.store.root(),
    );
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
    quiet: bool,
    cov_files: &[PathBuf],
    explicit_root: Option<&Path>,
) -> Result<()> {
    let printer = setup_timing(timing);
    let progress = CheckProgress::new(quiet);
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

    let (computed, entry_count) = extract_and_analyze(
        &ctx,
        &filter,
        folder.as_deref(),
        &progress,
        cov_files,
    )?;

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
