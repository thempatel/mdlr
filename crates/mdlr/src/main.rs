use anyhow::{Context, Result, bail};
use clap::Parser;
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};

mod cache;
mod cli;
mod config;
mod git;
mod json_output;
mod metrics_rows;
mod symbol_commands;
mod tag_commands;
mod walk;

use cache::{CacheStore, FileCacheEntry, Ignores, now_timestamp};
use cli::{Cli, Command, MetricsCommand, OutputFormat};
use git::GitChangeDetector;
use json_output::{
    build_bucketed_json, build_complexity_json, build_fan_metrics_json,
    build_file_loc_json, build_semantic_tags_json, build_struct_json,
};
use mdlr_core::{Graph, Unit, build as build_graph};
use mdlr_metrics::{
    BucketedMetrics, ComplexityMetrics, FileLocMetrics, StructMetrics,
    StructuralMetrics, TagMetrics, Thresholds,
    compute_with_hub_thresholds as compute_structural,
};
use metrics_rows::{MetricsBundle, collect_metric_rows};
use symbol_commands::{handle_get, handle_ls};
use tag_commands::{
    handle_tag_add, handle_tag_clear, handle_tag_list, handle_tag_remove,
    handle_tag_show, verify_symbol_exists,
};
use walk::SourceWalker;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Check { target, save, k, pretty, format, base } => {
            handle_check(target.as_deref(), save, k, pretty, format, base)
        }
        Command::Metrics { command } => handle_metrics(command),
        Command::Prompt => handle_prompt(),
        Command::Ls { path, kind, format } => handle_ls(&path, kind, format),
        Command::Get { symbol, format } => handle_get(&symbol, format),
        Command::Tag { symbol, add, remove, clear, list, format } => {
            handle_tag(symbol, add, remove, clear, list, format)
        }
        Command::Ignore { metric, symbol, remove, list } => {
            handle_ignore(metric, symbol, remove, list)
        }
    }
}

fn get_metric_descriptions() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "dag_density",
            "Ratio of edges to nodes in the dependency graph. High values indicate tightly coupled code; low values suggest isolated components.",
        ),
        (
            "fan_in",
            "Number of incoming dependencies to a unit. High values indicate core/shared code; very high may signal a bottleneck.",
        ),
        (
            "fan_out",
            "Number of outgoing dependencies from a unit. High values indicate a unit with many responsibilities that may need refactoring.",
        ),
        (
            "function_size",
            "Function size in lines of code. High values suggest functions that are hard to understand and test.",
        ),
        (
            "params",
            "Number of parameters on a function. High values (>4) often indicate a function doing too much or needing a parameter object.",
        ),
        (
            "cyclomatic",
            "Cyclomatic complexity (branches + 1) of a function. High values indicate complex control flow that is harder to test and maintain.",
        ),
        (
            "methods_per_struct",
            "Number of methods in a struct. High values may indicate a type with too many responsibilities.",
        ),
        (
            "lcom",
            "Lack of Cohesion of Methods (LCOM4). Counts connected components of methods sharing fields or calls. 1 = cohesive, 2+ = struct has unrelated groups and could be split.",
        ),
        (
            "file_loc",
            "Lines of code per file. High values indicate large files that may be hard to navigate and maintain.",
        ),
        (
            "tag_coverage",
            "Percentage of units with semantic tags applied. Low values indicate incomplete conceptual mapping of the codebase.",
        ),
        (
            "conceptual_fan_out",
            "Number of distinct semantic concepts a unit participates in. High values indicate mixed responsibilities across domains.",
        ),
        (
            "concept_scattering",
            "How spread out a concept is across files. High values indicate poor cohesion; the concept should be consolidated.",
        ),
        (
            "cross_concept_ratio",
            "Percentage of edges crossing concept boundaries. High values indicate tight coupling between different domains.",
        ),
    ]
}

fn handle_metrics(command: MetricsCommand) -> Result<()> {
    match command {
        MetricsCommand::Ls => {
            for (name, description) in get_metric_descriptions() {
                println!("{}", name);
                println!("  {}", description);
                println!();
            }
        }
        MetricsCommand::Get { name } => {
            let descriptions = get_metric_descriptions();
            let metric = descriptions.iter().find(|(n, _)| *n == name);

            match metric {
                Some((name, description)) => {
                    println!("{}", name);
                    println!("  {}", description);
                    println!();

                    let config = config::load()?;
                    if let Some(t) = config.thresholds.get(name) {
                        println!("thresholds:");
                        println!("  excellent  < {}", t.excellent);
                        println!("  good       < {}", t.good);
                        println!("  fair       < {}", t.fair);
                        println!("  poor       < {}", t.poor);
                        println!("  critical   >= {}", t.poor);
                    } else {
                        println!("(no thresholds defined)");
                    }
                }
                None => {
                    bail!(
                        "Unknown metric '{}'. Run 'mdlr metrics ls' to see available metrics.",
                        name
                    );
                }
            }
        }
    }

    Ok(())
}

fn handle_prompt() -> Result<()> {
    print!("{}", include_str!("prompt.md"));
    Ok(())
}

/// Represents what type of filter was specified
enum CheckFilter {
    /// No filter - analyze entire project
    None,
    /// Filter by file path
    File(std::path::PathBuf),
    /// Filter by directory path
    Directory(std::path::PathBuf),
    /// Filter by symbol ID
    Symbol(String),
}

/// Result of walking and extracting units from source files
struct WalkResult {
    units: Vec<Unit>,
    extracted_count: usize,
    cached_count: usize,
    entries_to_save: Vec<FileCacheEntry>,
    files_with_matching_symbols: HashSet<std::path::PathBuf>,
}

/// Result of collecting files to process
struct FileCollectionResult {
    /// All files discovered by the walker
    all_files: Vec<std::path::PathBuf>,
    /// Rust files that need extraction (not cached or cache stale).
    /// source_path contains the absolute path; units is empty until extraction.
    files_to_extract: Vec<FileCacheEntry>,
    /// Units loaded from cache
    cached_units: Vec<Unit>,
    /// Count of cached files
    cached_count: usize,
    /// Files that contain matching symbols (for symbol filter)
    files_with_matching_symbols: HashSet<std::path::PathBuf>,
}

/// Bundle of all computed metrics for a graph
struct ComputedMetrics {
    graph: Graph,
    structural: StructuralMetrics,
    complexity: ComplexityMetrics,
    struct_metrics: StructMetrics,
    file_loc: FileLocMetrics,
    tag_metrics: TagMetrics,
    has_staged: bool,
}

/// Context for the check command, bundling common resources
struct CheckContext {
    cwd: std::path::PathBuf,
    store: CacheStore,
    config: config::Config,
    walker: SourceWalker,
}

impl CheckContext {
    fn new() -> Result<Self> {
        let cwd = env::current_dir()?;
        let store = CacheStore::find_or_create(&cwd)?;
        let config = config::load()?;
        let walker = SourceWalker::new(store.root());

        Ok(CheckContext { cwd, store, config, walker })
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

/// Collect files to process, separating cached from uncached.
/// Uses git-based change detection: files in `git_changed` are considered stale.
fn collect_files_to_process(
    walker: &SourceWalker,
    filter: &CheckFilter,
    store: &CacheStore,
    git_changed: &HashSet<PathBuf>,
) -> Result<FileCollectionResult> {
    let mut result = FileCollectionResult {
        all_files: Vec::new(),
        files_to_extract: Vec::new(),
        cached_units: Vec::new(),
        cached_count: 0,
        files_with_matching_symbols: HashSet::new(),
    };

    for file_path in walker.walk() {
        // Collect all files before filtering
        result.all_files.push(file_path.clone());

        if !passes_path_filter(&file_path, filter) {
            continue;
        }

        // Only process Rust files for extraction
        if file_path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }

        let relative = file_path
            .strip_prefix(store.root())
            .unwrap_or(&file_path)
            .to_path_buf();

        // Check if file is changed according to git
        let is_git_changed = git_changed.contains(&file_path);

        let cached_entry = store.load_entry(&file_path)?;

        if let Some(entry) = cached_entry {
            if !is_git_changed {
                // Cache is valid (file not changed in git)
                result.cached_count += 1;

                // For symbol filter, check if any unit matches
                if let CheckFilter::Symbol(symbol_id) = filter {
                    if entry.units.iter().any(|u| u.id == *symbol_id) {
                        result
                            .files_with_matching_symbols
                            .insert(relative.clone());
                    }
                }

                result.cached_units.extend(entry.units);
                continue;
            }

            // Cache is stale (file changed in git), need to re-extract
            result.files_to_extract.push(FileCacheEntry {
                source_path: file_path,
                units: Vec::new(),
                cached_at: 0,
            });
        } else {
            // No cache entry, need to extract
            result.files_to_extract.push(FileCacheEntry {
                source_path: file_path,
                units: Vec::new(),
                cached_at: 0,
            });
        }
    }

    Ok(result)
}

/// Walk source files and extract units, using cache when available.
/// Uses git-based change detection for staleness.
fn walk_and_extract_units(
    walker: &SourceWalker,
    filter: &CheckFilter,
    store: &CacheStore,
    save: bool,
    git_changed: &HashSet<PathBuf>,
) -> Result<WalkResult> {
    // First pass: collect files and load from cache
    let collection =
        collect_files_to_process(walker, filter, store, git_changed)?;

    let mut result = WalkResult {
        units: collection.cached_units,
        extracted_count: 0,
        cached_count: collection.cached_count,
        entries_to_save: Vec::new(),
        files_with_matching_symbols: collection.files_with_matching_symbols,
    };

    // If no files need extraction, we're done
    if collection.files_to_extract.is_empty() {
        return Ok(result);
    }

    // Shell out to mdlr-extract-rust for extraction
    let extracted = extract_rust(&collection.files_to_extract, store.root())?;

    for (entry, file_entry) in
        collection.files_to_extract.iter().zip(extracted.iter())
    {
        let units = match file_entry {
            Some(fe) => &fe.units,
            None => continue,
        };

        result.extracted_count += 1;

        let relative = entry
            .source_path
            .strip_prefix(store.root())
            .unwrap_or(&entry.source_path)
            .to_path_buf();

        // For symbol filter, check if any unit matches
        if let CheckFilter::Symbol(symbol_id) = filter {
            if units.iter().any(|u| u.id == *symbol_id) {
                result.files_with_matching_symbols.insert(relative.clone());
            }
        }

        if save {
            let save_entry = FileCacheEntry {
                source_path: relative,
                units: units.clone(),
                cached_at: now_timestamp(),
            };
            result.entries_to_save.push(save_entry);
        }
        result.units.extend(units.clone());
    }

    Ok(result)
}

/// Find the `mdlr-extract-rust` binary, checking next to our own binary first.
fn find_extract_rust_binary() -> Result<PathBuf> {
    if let Ok(current_exe) = env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let sibling = dir.join("mdlr-extract-rust");
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }
    // Check if it's on PATH
    if let Ok(output) =
        std::process::Command::new("which").arg("mdlr-extract-rust").output()
    {
        if output.status.success() {
            let path =
                String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    bail!(
        "Could not find mdlr-extract-rust binary. \
         Build it with: cargo install --path crates/mdlr-extract-rust"
    );
}

/// Shell out to `mdlr-extract-rust` in standalone mode to extract units.
///
/// Creates a mapping file, invokes `mdlr-extract-rust --manifest-path <path>
/// --mapping <path>` once, then loads the resulting JSON.
fn extract_rust(
    files: &[FileCacheEntry],
    workspace_root: &Path,
) -> Result<Vec<Option<FileCacheEntry>>> {
    let tmp_dir = tempfile::tempdir()?;

    // Build mapping: relative source path → temp output path
    // Using relative paths (relative to workspace_root) so that unit.file
    // fields in the extracted output are project-relative, not absolute.
    let mut mapping: HashMap<String, String> = HashMap::new();
    for (i, entry) in files.iter().enumerate() {
        let relative = entry
            .source_path
            .strip_prefix(workspace_root)
            .unwrap_or(&entry.source_path);
        let source_key = relative.to_string_lossy().to_string();
        let output_path = tmp_dir.path().join(format!("{}.json", i));
        mapping.insert(source_key, output_path.to_string_lossy().to_string());
    }

    // Write mapping file
    let mapping_path = tmp_dir.path().join("mapping.json");
    std::fs::write(&mapping_path, serde_json::to_string(&mapping)?)?;

    // Find mdlr-extract-rust binary
    let extract_bin = find_extract_rust_binary()?;

    // Find the workspace Cargo.toml
    let manifest_path = workspace_root.join("Cargo.toml");
    if !manifest_path.exists() {
        bail!("No Cargo.toml found at {}", manifest_path.display());
    }

    // Single invocation of mdlr-extract-rust in standalone mode.
    // Inherit stderr so cargo progress bars and diagnostics show through.
    let status = std::process::Command::new(&extract_bin)
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--mapping")
        .arg(&mapping_path)
        .current_dir(workspace_root)
        .stderr(std::process::Stdio::inherit())
        .status()
        .context("Failed to run mdlr-extract-rust")?;

    if !status.success() {
        eprintln!("Warning: HIR extraction failed");
    }

    // Load results from temp files
    let mut results = Vec::with_capacity(files.len());
    for i in 0..files.len() {
        let output_path = tmp_dir.path().join(format!("{}.json", i));
        if output_path.exists() {
            let content = std::fs::read_to_string(&output_path)?;
            let file_entry: FileCacheEntry = serde_json::from_str(&content)?;
            results.push(Some(file_entry));
        } else {
            results.push(None);
        }
    }

    Ok(results)
}

/// Check if a file path passes the filter
fn passes_path_filter(file_path: &Path, filter: &CheckFilter) -> bool {
    match filter {
        CheckFilter::File(filter_path) => file_path == *filter_path,
        CheckFilter::Directory(filter_path) => {
            file_path.starts_with(filter_path)
        }
        CheckFilter::Symbol(_) | CheckFilter::None => true,
    }
}
/// Save cache entries based on filter type
fn save_cache_entries(
    store: &CacheStore,
    filter: &CheckFilter,
    entries: Vec<FileCacheEntry>,
    matching_files: &HashSet<std::path::PathBuf>,
) -> Result<()> {
    match filter {
        CheckFilter::Symbol(_) => {
            // Only save files that contain matching symbols
            for entry in entries {
                if matching_files.contains(&entry.source_path) {
                    store.save_entry(&entry)?;
                }
            }
        }
        _ => {
            // Save all entries (already filtered by path)
            for entry in entries {
                store.save_entry(&entry)?;
            }
        }
    }
    // Commit any staged tag changes
    store.commit_staged_tags()?;
    Ok(())
}

/// Compute all metrics from units
fn compute_all_metrics(
    units: Vec<Unit>,
    store: &CacheStore,
    config: &config::Config,
) -> ComputedMetrics {
    let graph = build_graph(units);
    let structural = compute_structural(
        &graph,
        config.hub.min_fan_in,
        config.hub.min_fan_out,
    );
    let complexity = ComplexityMetrics::compute(&graph);
    let struct_metrics = StructMetrics::compute(&graph);
    let file_loc = FileLocMetrics::compute(&graph);
    let semantic_tags = store.load_tags_with_staged().unwrap_or_default();
    let has_staged = store.has_staged_tags();

    // Convert cache SemanticTags to metrics SemanticTags
    let metrics_tags =
        mdlr_metrics::SemanticTags { tags: semantic_tags.tags.clone() };
    let tag_metrics = TagMetrics::compute(&graph, &metrics_tags);

    ComputedMetrics {
        graph,
        structural,
        complexity,
        struct_metrics,
        file_loc,
        tag_metrics,
        has_staged,
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
        tag_metrics: &computed.tag_metrics,
    };
    let symbol_filter = get_symbol_filter(filter);
    let ignores = store.load_ignores().unwrap_or_default();
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

    if computed.has_staged {
        eprintln!("(staged tag changes pending - use --save to commit)");
    }

    Ok(())
}

/// Format and print JSON output
fn format_json_output(
    computed: &ComputedMetrics,
    config: &config::Config,
    extracted_count: usize,
    cached_count: usize,
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

    let output = serde_json::json!({
        "files": {
            "extracted": extracted_count,
            "cached": cached_count,
        },
        "units": computed.graph.units.len(),
        "edges": computed.graph.edges.len(),
        "metrics": {
            "dag_density": build_bucketed_json(&bucketed.dag_density),
            "fan_in": build_fan_metrics_json(&bucketed.fan_in, &computed.structural.fan_in.distribution),
            "fan_out": build_fan_metrics_json(&bucketed.fan_out, &computed.structural.fan_out.distribution),
            "complexity": build_complexity_json(&computed.complexity),
            "struct": build_struct_json(&computed.struct_metrics),
            "file_loc": build_file_loc_json(&computed.file_loc),
            "semantic_tags": build_semantic_tags_json(&computed.tag_metrics),
        }
    });
    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

/// Build JSON output for a specific symbol
fn build_symbol_json(
    computed: &ComputedMetrics,
    config: &config::Config,
    symbol_id: &str,
) -> serde_json::Value {
    let mut metrics = serde_json::Map::new();

    // Fan-in and fan-out from structural metrics
    if let Some((_, value)) = computed
        .structural
        .fan_in
        .distribution
        .iter()
        .find(|(name, _)| name == symbol_id)
    {
        let bucket = config.thresholds.fan_in_max.evaluate(*value as f64);
        metrics.insert(
            "fan_in".to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }

    if let Some((_, value)) = computed
        .structural
        .fan_out
        .distribution
        .iter()
        .find(|(name, _)| name == symbol_id)
    {
        let bucket = config.thresholds.fan_out_max.evaluate(*value as f64);
        metrics.insert(
            "fan_out".to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }

    // Complexity metrics
    if let Some((_, value)) = computed
        .complexity
        .size
        .distribution
        .iter()
        .find(|(name, _)| name == symbol_id)
    {
        let bucket = config.thresholds.function_size.evaluate(*value as f64);
        metrics.insert(
            "function_size".to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }

    if let Some((_, value)) = computed
        .complexity
        .params
        .distribution
        .iter()
        .find(|(name, _)| name == symbol_id)
    {
        let bucket = config.thresholds.params.evaluate(*value as f64);
        metrics.insert(
            "params".to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }

    if let Some((_, value)) = computed
        .complexity
        .cyclomatic
        .distribution
        .iter()
        .find(|(name, _)| name == symbol_id)
    {
        let bucket = config.thresholds.cyclomatic.evaluate(*value as f64);
        metrics.insert(
            "cyclomatic".to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }

    // Struct metrics
    if let Some((_, value)) = computed
        .struct_metrics
        .methods_per_struct
        .distribution
        .iter()
        .find(|(name, _)| name == symbol_id)
    {
        let bucket =
            config.thresholds.methods_per_struct.evaluate(*value as f64);
        metrics.insert(
            "methods_per_struct".to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }

    if let Some((_, value)) = computed
        .struct_metrics
        .lcom
        .distribution
        .iter()
        .find(|(name, _)| name == symbol_id)
    {
        let bucket = config.thresholds.lcom.evaluate(*value as f64);
        metrics.insert(
            "lcom".to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }

    serde_json::json!({
        "symbol": symbol_id,
        "metrics": metrics
    })
}

fn handle_check(
    target: Option<&str>,
    save: bool,
    k: i32,
    pretty: bool,
    format: OutputFormat,
    base: Option<String>,
) -> Result<()> {
    let ctx = CheckContext::new()?;

    // Initialize git-based change detection
    let git_detector = GitChangeDetector::open(
        ctx.store.root(),
        &ctx.config.git.main_branch,
    )?;
    let git_changed = git_detector.detect_changes(
        base.as_deref(),
        ctx.config.git.base_commit.as_deref(),
    )?;

    let filter = parse_check_filter(target, &ctx.cwd);
    let walk_result = walk_and_extract_units(
        &ctx.walker,
        &filter,
        &ctx.store,
        save,
        &git_changed,
    )?;

    let units = walk_result.units;

    // Validate symbol exists before building graph (bail if not found)
    if let CheckFilter::Symbol(symbol_id) = &filter {
        if !units.iter().any(|u| u.id == *symbol_id) {
            bail!(
                "Symbol '{}' not found. Run 'mdlr ls' to see available symbols.",
                symbol_id
            );
        }
    }

    if save {
        save_cache_entries(
            &ctx.store,
            &filter,
            walk_result.entries_to_save,
            &walk_result.files_with_matching_symbols,
        )?;
    }

    // Build full graph with all units to capture all edges (including callers)
    let computed = compute_all_metrics(units, &ctx.store, &ctx.config);

    // Filter is applied at output time, not graph construction time
    match format {
        OutputFormat::Text => format_text_output(
            &computed,
            &ctx.config,
            k,
            pretty,
            &filter,
            &ctx.store,
        ),
        OutputFormat::Json => format_json_output(
            &computed,
            &ctx.config,
            walk_result.extracted_count,
            walk_result.cached_count,
            &filter,
        ),
    }
}

fn handle_tag(
    symbol: Option<String>,
    add: Vec<String>,
    remove: Option<String>,
    clear: bool,
    list: bool,
    format: OutputFormat,
) -> Result<()> {
    let store = CacheStore::open(Path::new("."))?;

    if list {
        return handle_tag_list(&store, format);
    }

    let symbol = symbol.ok_or_else(|| {
        anyhow::anyhow!("Symbol ID is required. Use 'mdlr tag --list' to see all tags, or specify a symbol.")
    })?;

    verify_symbol_exists(&store, &symbol)?;

    if clear {
        return handle_tag_clear(&store, &symbol);
    }

    if let Some(ref tag) = remove {
        return handle_tag_remove(&store, &symbol, tag);
    }

    if !add.is_empty() {
        return handle_tag_add(&store, &symbol, &add);
    }

    handle_tag_show(&store, &symbol, format)
}

/// Valid metric names that can be ignored
const VALID_METRICS: &[&str] = &[
    "fan_in",
    "fan_out",
    "function_size",
    "params",
    "cyclomatic",
    "methods_per_struct",
    "lcom",
    "file_loc",
];

// TODO: Make it so that agents cannot ignore, but humans can
fn handle_ignore(
    metric: Option<String>,
    symbol: Option<String>,
    remove: bool,
    list: bool,
) -> Result<()> {
    let store = CacheStore::open(Path::new("."))?;

    if list {
        return handle_ignore_list(&store);
    }

    let metric = metric.ok_or_else(|| {
        anyhow::anyhow!(
            "Metric name is required. Valid metrics: {}",
            VALID_METRICS.join(", ")
        )
    })?;

    // Validate metric name
    if !VALID_METRICS.contains(&metric.as_str()) {
        bail!(
            "Unknown metric '{}'. Valid metrics: {}",
            metric,
            VALID_METRICS.join(", ")
        );
    }

    let symbol =
        symbol.ok_or_else(|| anyhow::anyhow!("Symbol ID is required."))?;

    if remove {
        handle_ignore_remove(&store, &metric, &symbol)
    } else {
        handle_ignore_add(&store, &metric, &symbol)
    }
}

fn handle_ignore_list(store: &CacheStore) -> Result<()> {
    let ignores = store.load_ignores()?;

    if ignores.is_empty() {
        println!("No ignores configured.");
        return Ok(());
    }

    // Collect and sort for consistent output
    let mut entries: Vec<_> = ignores.ignores.iter().collect();
    entries.sort_by_key(|(symbol, _)| *symbol);

    for (symbol, metrics) in entries {
        for metric in metrics {
            println!("{}\t{}", metric, symbol);
        }
    }

    Ok(())
}

fn handle_ignore_add(
    store: &CacheStore,
    metric: &str,
    symbol: &str,
) -> Result<()> {
    let mut ignores = store.load_ignores()?;

    if ignores.is_ignored(symbol, metric) {
        println!("Already ignoring {} for {}", metric, symbol);
        return Ok(());
    }

    ignores.add(symbol, metric);
    store.save_ignores(&ignores)?;
    println!("Ignoring {} for {}", metric, symbol);

    Ok(())
}

fn handle_ignore_remove(
    store: &CacheStore,
    metric: &str,
    symbol: &str,
) -> Result<()> {
    let mut ignores = store.load_ignores()?;

    if !ignores.remove(symbol, metric) {
        println!("No ignore found for {} on {}", metric, symbol);
        return Ok(());
    }

    store.save_ignores(&ignores)?;
    println!("Removed ignore for {} on {}", metric, symbol);

    Ok(())
}
