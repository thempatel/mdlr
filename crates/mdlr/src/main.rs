use anyhow::{Result, bail};
use clap::Parser;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

mod cache;
mod cli;
mod config;
mod json_output;
mod metrics_rows;
mod symbol_commands;
mod tag_commands;
mod walk;

use cache::{CacheStore, FileCacheEntry, get_file_metadata, now_timestamp};
use cli::{Cli, Command, MetricsCommand, OutputFormat};
use json_output::{
    build_bucketed_json, build_complexity_json, build_fan_metrics_json,
    build_file_loc_json, build_semantic_tags_json, build_struct_json,
};
use mdlr_core::{CallResolver, Graph, Unit, build as build_graph};
use mdlr_extract_rust::{
    CargoWorkspace, ResolutionContext, extractor_for_path,
};
use mdlr_metrics::{
    BucketedMetrics, ComplexityMetrics, FileLocMetrics, StructMetrics,
    StructuralMetrics, TagMetrics, Thresholds, compute as compute_structural,
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
        Command::Check { target, save, k, pretty, format } => {
            handle_check(target.as_deref(), save, k, pretty, format)
        }
        Command::Metrics { command } => handle_metrics(command),
        Command::Prompt => handle_prompt(),
        Command::Ls { path, kind, format } => handle_ls(&path, kind, format),
        Command::Get { symbol, format } => handle_get(&symbol, format),
        Command::Tag { symbol, add, remove, clear, list, format } => {
            handle_tag(symbol, add, remove, clear, list, format)
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
            "Lack of Cohesion of Methods. High values indicate methods don't share state, suggesting the struct could be split.",
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
    resolution_ctx: Option<ResolutionContext>,
}

impl CheckContext {
    fn new() -> Result<Self> {
        let cwd = env::current_dir()?;
        let store = CacheStore::find_or_create(&cwd)?;
        let config = config::load()?;
        let walker = SourceWalker::new(store.root());
        let resolution_ctx = CargoWorkspace::discover(store.root())
            .ok()
            .map(ResolutionContext::build);

        Ok(CheckContext { cwd, store, config, walker, resolution_ctx })
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

/// Walk source files and extract units, using cache when available
fn walk_and_extract_units(
    walker: &SourceWalker,
    filter: &CheckFilter,
    store: &CacheStore,
    save: bool,
    resolution_ctx: Option<&ResolutionContext>,
) -> Result<WalkResult> {
    let mut result = WalkResult {
        units: Vec::new(),
        extracted_count: 0,
        cached_count: 0,
        entries_to_save: Vec::new(),
        files_with_matching_symbols: HashSet::new(),
    };

    for file_path in walker.walk() {
        if !passes_path_filter(&file_path, filter) {
            continue;
        }

        let relative = file_path
            .strip_prefix(store.root())
            .unwrap_or(&file_path)
            .to_path_buf();

        let Some(units) = process_file(
            &file_path,
            &relative,
            store,
            save,
            resolution_ctx,
            &mut result,
        )?
        else {
            continue;
        };

        // For symbol filter, check if any unit matches and track the file
        if let CheckFilter::Symbol(symbol_id) = filter {
            if units.iter().any(|u| u.id == *symbol_id) {
                result.files_with_matching_symbols.insert(relative);
            }
        }

        result.units.extend(units);
    }

    Ok(result)
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

/// Build a cache entry for a file
fn build_cache_entry(
    relative: &Path,
    mtime: u64,
    size: u64,
    units: Vec<Unit>,
) -> FileCacheEntry {
    FileCacheEntry {
        source_path: relative.to_path_buf(),
        mtime,
        size,
        units,
        cached_at: now_timestamp(),
    }
}

/// Process a single file, returning its units if successful
fn process_file(
    file_path: &Path,
    relative: &Path,
    store: &CacheStore,
    save: bool,
    resolution_ctx: Option<&ResolutionContext>,
    result: &mut WalkResult,
) -> Result<Option<Vec<Unit>>> {
    let cached_entry = store.load_entry(file_path)?;

    if let Some(entry) = cached_entry {
        process_cached_file(
            file_path,
            relative,
            entry,
            save,
            resolution_ctx,
            result,
        )
    } else {
        Ok(process_uncached_file(
            file_path,
            relative,
            save,
            resolution_ctx,
            result,
        ))
    }
}

/// Process a file that has a cache entry
fn process_cached_file(
    file_path: &Path,
    relative: &Path,
    entry: FileCacheEntry,
    save: bool,
    resolution_ctx: Option<&ResolutionContext>,
    result: &mut WalkResult,
) -> Result<Option<Vec<Unit>>> {
    let current_meta = get_file_metadata(file_path)?;

    if entry.mtime == current_meta.mtime && entry.size == current_meta.size {
        result.cached_count += 1;
        return Ok(Some(entry.units));
    }

    // Cache is stale, re-extract
    let Some(units) = extract_units_from_file(file_path, resolution_ctx)
    else {
        return Ok(None);
    };

    result.extracted_count += 1;
    if save {
        result.entries_to_save.push(build_cache_entry(
            relative,
            current_meta.mtime,
            current_meta.size,
            units.clone(),
        ));
    }
    Ok(Some(units))
}

/// Process a file that has no cache entry
fn process_uncached_file(
    file_path: &Path,
    relative: &Path,
    save: bool,
    _resolution_ctx: Option<&ResolutionContext>,
    result: &mut WalkResult,
) -> Option<Vec<Unit>> {
    let extractor = extractor_for_path(file_path)?;
    let current_meta = get_file_metadata(file_path).ok()?;

    match extract_file(file_path, extractor.as_ref()) {
        Ok(units) => {
            result.extracted_count += 1;
            if save {
                result.entries_to_save.push(build_cache_entry(
                    relative,
                    current_meta.mtime,
                    current_meta.size,
                    units.clone(),
                ));
            }
            Some(units)
        }
        Err(e) => {
            eprintln!(
                "Warning: Failed to extract {}: {}",
                file_path.display(),
                e
            );
            None
        }
    }
}

/// Helper to extract units from a file, returning None on failure
fn extract_units_from_file(
    file_path: &Path,
    _resolution_ctx: Option<&ResolutionContext>,
) -> Option<Vec<Unit>> {
    let extractor = extractor_for_path(file_path)?;
    match extract_file(file_path, extractor.as_ref()) {
        Ok(units) => Some(units),
        Err(e) => {
            eprintln!(
                "Warning: Failed to extract {}: {}",
                file_path.display(),
                e
            );
            None
        }
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
    resolution_ctx: Option<&ResolutionContext>,
    store: &CacheStore,
) -> ComputedMetrics {
    let resolver: Option<&dyn CallResolver> =
        resolution_ctx.map(|r| r as &dyn CallResolver);
    let graph = build_graph(units, resolver);
    let structural = compute_structural(&graph);
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
) -> Result<()> {
    let bundle = MetricsBundle {
        structural: &computed.structural,
        complexity: &computed.complexity,
        struct_metrics: &computed.struct_metrics,
        file_loc: &computed.file_loc,
        tag_metrics: &computed.tag_metrics,
    };
    let symbol_filter = get_symbol_filter(filter);
    let rows = collect_metric_rows(&bundle, config, k, symbol_filter);

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
            "fan_in": build_fan_metrics_json(&bucketed.fan_in),
            "fan_out": build_fan_metrics_json(&bucketed.fan_out),
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
        let bucket = config.thresholds.lcom.evaluate(*value);
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
) -> Result<()> {
    let ctx = CheckContext::new()?;

    let filter = parse_check_filter(target, &ctx.cwd);
    let walk_result = walk_and_extract_units(
        &ctx.walker,
        &filter,
        &ctx.store,
        save,
        ctx.resolution_ctx.as_ref(),
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
    let computed =
        compute_all_metrics(units, ctx.resolution_ctx.as_ref(), &ctx.store);

    // Filter is applied at output time, not graph construction time
    match format {
        OutputFormat::Text => {
            format_text_output(&computed, &ctx.config, k, pretty, &filter)
        }
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

fn extract_file(
    abs_path: &Path,
    extractor: &dyn mdlr_core::Extractor,
) -> Result<Vec<Unit>> {
    let source = fs::read_to_string(abs_path)?;
    extractor.extract(&source, abs_path)
}
