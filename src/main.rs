use anyhow::{bail, Result};
use clap::Parser;
use std::io::Write;
use mdlr::cache::{get_file_metadata, now_timestamp, CacheStore, FileCacheEntry};
use mdlr::cli::{Cli, Command, OutputFormat};
use mdlr::config;
use mdlr::extract::{extractor_for_path, Extractor};
use mdlr::graph::{Edge, EdgeKind, Graph, Unit, UnitKind};
use mdlr::metrics::{BucketedMetrics, ComplexityMetrics, ImplMetrics, TagMetrics};
use mdlr::resolve::{CargoWorkspace, ResolutionContext};
use mdlr::walk::SourceWalker;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Check { target, save, k, pretty, format } => handle_check(target.as_deref(), save, k, pretty, format),
        Command::Metrics => handle_metrics(),
        Command::Prompt => handle_prompt(),
        Command::Ls { path, kind, format } => handle_ls(&path, kind, format),
        Command::Get { symbol, format } => handle_get(&symbol, format),
        Command::Tag {
            symbol,
            add,
            remove,
            clear,
            list,
            format,
        } => handle_tag(symbol, add, remove, clear, list, format),
    }
}

fn handle_metrics() -> Result<()> {
    let metrics = [
        ("dag_density", "Ratio of edges to nodes in the dependency graph. High values indicate tightly coupled code; low values suggest isolated components."),
        ("fan_in", "Number of incoming dependencies to a unit. High values indicate core/shared code; very high may signal a bottleneck."),
        ("fan_out", "Number of outgoing dependencies from a unit. High values indicate a unit with many responsibilities that may need refactoring."),
        ("function_size", "Function size in lines of code. High values suggest functions that are hard to understand and test."),
        ("params", "Number of parameters on a function. High values (>4) often indicate a function doing too much or needing a parameter object."),
        ("cyclomatic", "Cyclomatic complexity (branches + 1) of a function. High values indicate complex control flow that is harder to test and maintain."),
        ("methods_per_impl", "Number of methods in an impl block. High values may indicate a type with too many responsibilities."),
        ("traits_per_type", "Number of traits implemented by a type. High values may indicate a versatile type or one trying to do too much."),
        ("lcom", "Lack of Cohesion of Methods. High values indicate methods don't share state, suggesting the impl could be split."),
        ("tag_coverage", "Percentage of units with semantic tags applied. Low values indicate incomplete conceptual mapping of the codebase."),
        ("conceptual_fan_out", "Number of distinct semantic concepts a unit participates in. High values indicate mixed responsibilities across domains."),
        ("concept_scattering", "How spread out a concept is across files. High values indicate poor cohesion; the concept should be consolidated."),
        ("cross_concept_ratio", "Percentage of edges crossing concept boundaries. High values indicate tight coupling between different domains."),
    ];

    for (name, description) in metrics {
        println!("{}", name);
        println!("  {}", description);
        println!();
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

fn handle_check(target: Option<&str>, save: bool, k: i32, pretty: bool, format: OutputFormat) -> Result<()> {
    let cwd = env::current_dir()?;
    let store = CacheStore::find_or_create(&cwd)?;
    let config = config::load()?;
    let walker = SourceWalker::new(store.root());

    // Try to build a resolution context for enhanced call resolution
    let resolution_ctx = CargoWorkspace::discover(store.root())
        .ok()
        .map(ResolutionContext::build);

    // Determine what kind of filter we have
    let filter = if let Some(target_str) = target {
        let target_path = if Path::new(target_str).is_absolute() {
            Path::new(target_str).to_path_buf()
        } else {
            cwd.join(target_str)
        };

        if target_path.exists() {
            // It's a path
            let canonical = target_path.canonicalize().unwrap_or(target_path);
            if canonical.is_file() {
                CheckFilter::File(canonical)
            } else {
                CheckFilter::Directory(canonical)
            }
        } else {
            // Treat as symbol ID
            CheckFilter::Symbol(target_str.to_string())
        }
    } else {
        CheckFilter::None
    };

    let mut all_units: Vec<Unit> = Vec::new();
    let mut extracted_count = 0;
    let mut cached_count = 0;

    // Track entries to save if --save is used
    let mut entries_to_save: Vec<FileCacheEntry> = Vec::new();

    // Track files that contain matching symbols (for symbol filter + save)
    let mut files_with_matching_symbols: HashSet<std::path::PathBuf> = HashSet::new();

    for file_path in walker.walk() {
        // Apply path filter if specified (skip symbol filter for now, applied later)
        match &filter {
            CheckFilter::File(filter_path) => {
                if file_path != *filter_path {
                    continue;
                }
            }
            CheckFilter::Directory(filter_path) => {
                if !file_path.starts_with(filter_path) {
                    continue;
                }
            }
            CheckFilter::Symbol(_) | CheckFilter::None => {
                // No path filtering needed
            }
        }

        let relative = file_path
            .strip_prefix(store.root())
            .unwrap_or(&file_path)
            .to_path_buf();

        // Try to load from cache first
        let cached_entry = store.load_entry(&file_path)?;

        let units = if let Some(entry) = cached_entry {
            // Check if cache is still valid
            let current_meta = get_file_metadata(&file_path)?;
            if entry.mtime == current_meta.mtime && entry.size == current_meta.size {
                cached_count += 1;
                entry.units
            } else {
                // Cache is stale, re-extract
                if let Some(extractor) = extractor_for_path(&file_path) {
                    match extract_file(&file_path, extractor.as_ref(), resolution_ctx.as_ref()) {
                        Ok(units) => {
                            extracted_count += 1;
                            // Defer save decision until we know if file matches symbol filter
                            if save {
                                entries_to_save.push(FileCacheEntry {
                                    source_path: relative.clone(),
                                    mtime: current_meta.mtime,
                                    size: current_meta.size,
                                    units: units.clone(),
                                    cached_at: now_timestamp(),
                                });
                            }
                            units
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to extract {}: {}", file_path.display(), e);
                            continue;
                        }
                    }
                } else {
                    continue;
                }
            }
        } else {
            // No cache entry, extract fresh
            if let Some(extractor) = extractor_for_path(&file_path) {
                let current_meta = get_file_metadata(&file_path)?;
                match extract_file(&file_path, extractor.as_ref(), resolution_ctx.as_ref()) {
                    Ok(units) => {
                        extracted_count += 1;
                        // Defer save decision until we know if file matches symbol filter
                        if save {
                            entries_to_save.push(FileCacheEntry {
                                source_path: relative.clone(),
                                mtime: current_meta.mtime,
                                size: current_meta.size,
                                units: units.clone(),
                                cached_at: now_timestamp(),
                            });
                        }
                        units
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to extract {}: {}", file_path.display(), e);
                        continue;
                    }
                }
            } else {
                continue;
            }
        };

        // For symbol filter, check if any unit matches and track the file
        if let CheckFilter::Symbol(ref symbol_id) = filter {
            let has_match = units.iter().any(|u| u.id == *symbol_id);
            if has_match {
                files_with_matching_symbols.insert(relative.clone());
            }
        }

        all_units.extend(units);
    }

    // Apply symbol filter to units
    if let CheckFilter::Symbol(ref symbol_id) = filter {
        all_units.retain(|u| u.id == *symbol_id);
        if all_units.is_empty() {
            bail!("Symbol '{}' not found. Run 'mdlr ls' to see available symbols.", symbol_id);
        }
    }

    // Save entries and commit staged tags if --save flag was provided
    // Only save entries that match the filter
    if save {
        match &filter {
            CheckFilter::Symbol(_) => {
                // Only save files that contain matching symbols
                for entry in entries_to_save {
                    if files_with_matching_symbols.contains(&entry.source_path) {
                        store.save_entry(&entry)?;
                    }
                }
            }
            _ => {
                // Save all entries (already filtered by path)
                for entry in entries_to_save {
                    store.save_entry(&entry)?;
                }
            }
        }
        // Commit any staged tag changes
        store.commit_staged_tags()?;
    }

    let graph = build_graph(all_units, resolution_ctx.as_ref());
    let metrics = mdlr::metrics::compute(&graph);
    let complexity = ComplexityMetrics::compute(&graph);
    let impl_metrics = ImplMetrics::compute(&graph);
    // Load tags with staged changes overlaid
    let semantic_tags = store.load_tags_with_staged()?;
    let has_staged = store.has_staged_tags();
    let tag_metrics = TagMetrics::compute(&graph, &semantic_tags);

    match format {
        OutputFormat::Text => {
            let take = |n: usize| if k < 0 { n } else { k as usize };

            // Collect all rows: (metric, symbol, value)
            let mut rows: Vec<(String, String, String)> = Vec::new();

            // Fan-out opportunities
            for (name, count) in metrics.fan_out.distribution.iter().take(take(metrics.fan_out.distribution.len())) {
                if *count > 0 {
                    rows.push(("fan_out".to_string(), name.clone(), count.to_string()));
                }
            }

            // Fan-in opportunities
            for (name, count) in metrics.fan_in.distribution.iter().take(take(metrics.fan_in.distribution.len())) {
                if *count > 0 {
                    rows.push(("fan_in".to_string(), name.clone(), count.to_string()));
                }
            }

            // Function size opportunities
            for (name, size) in complexity.size.distribution.iter().take(take(complexity.size.distribution.len())) {
                if *size > 1 {
                    rows.push(("function_size".to_string(), name.clone(), size.to_string()));
                }
            }

            // Parameter count opportunities
            for (name, params) in complexity.params.distribution.iter().take(take(complexity.params.distribution.len())) {
                if *params > 0 {
                    rows.push(("params".to_string(), name.clone(), params.to_string()));
                }
            }

            // Cyclomatic complexity opportunities
            for (name, cc) in complexity.cyclomatic.distribution.iter().take(take(complexity.cyclomatic.distribution.len())) {
                if *cc > 1 {
                    rows.push(("cyclomatic".to_string(), name.clone(), cc.to_string()));
                }
            }

            // Methods per impl opportunities
            for (name, count) in impl_metrics.methods_per_impl.distribution.iter().take(take(impl_metrics.methods_per_impl.distribution.len())) {
                if *count > 0 {
                    rows.push(("methods_per_impl".to_string(), name.clone(), count.to_string()));
                }
            }

            // Traits per type opportunities
            for (name, count) in impl_metrics.traits_per_type.distribution.iter().take(take(impl_metrics.traits_per_type.distribution.len())) {
                if *count > 0 {
                    rows.push(("traits_per_type".to_string(), name.clone(), count.to_string()));
                }
            }

            // LCOM opportunities
            for (name, lcom) in impl_metrics.lcom.distribution.iter().take(take(impl_metrics.lcom.distribution.len())) {
                if *lcom > 0.0 {
                    rows.push(("lcom".to_string(), name.clone(), format!("{:.2}", lcom)));
                }
            }

            // Conceptual metrics (if tags exist)
            if let Some(ref conceptual) = tag_metrics.conceptual {
                for (name, count) in conceptual.conceptual_fan_out.top.iter().take(take(conceptual.conceptual_fan_out.top.len())) {
                    if *count > 1 {
                        rows.push(("conceptual_fan_out".to_string(), name.clone(), count.to_string()));
                    }
                }

                for scatter in conceptual.concept_scattering.iter().take(take(conceptual.concept_scattering.len())) {
                    if scatter.file_count > 1 {
                        rows.push(("concept_scattering".to_string(), scatter.tag.clone(), format!("{:.2}", scatter.scatter_ratio)));
                    }
                }
            }

            // Print output
            if pretty {
                let mut tw = tabwriter::TabWriter::new(vec![]);
                writeln!(tw, "metric\tsymbol\tvalue")?;
                for (metric, symbol, value) in &rows {
                    writeln!(tw, "{}\t{}\t{}", metric, symbol, value)?;
                }
                tw.flush()?;
                print!("{}", String::from_utf8_lossy(&tw.into_inner()?));
            } else {
                println!("metric\tsymbol\tvalue");
                for (metric, symbol, value) in &rows {
                    println!("{}\t{}\t{}", metric, symbol, value);
                }
            }

            if has_staged {
                eprintln!("(staged tag changes pending - use --save to commit)");
            }
        }
        OutputFormat::Json => {
            let bucketed = BucketedMetrics::from_metrics(&metrics, &config);

            let namespace_distribution: serde_json::Map<String, serde_json::Value> = tag_metrics
                .namespace_distribution
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                .collect();

            let namespace_values: serde_json::Map<String, serde_json::Value> = tag_metrics
                .namespace_values
                .iter()
                .map(|(ns, values)| {
                    let values_map: serde_json::Map<String, serde_json::Value> = values
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                        .collect();
                    (ns.clone(), serde_json::Value::Object(values_map))
                })
                .collect();

            // Build conceptual metrics JSON if present
            let conceptual_json = tag_metrics.conceptual.as_ref().map(|c| {
                let scattering: Vec<_> = c
                    .concept_scattering
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "tag": s.tag,
                            "unit_count": s.unit_count,
                            "file_count": s.file_count,
                            "scatter_ratio": s.scatter_ratio,
                        })
                    })
                    .collect();

                let cross_concept_by_ns: serde_json::Map<String, serde_json::Value> = c
                    .cross_concept_edges
                    .by_namespace
                    .iter()
                    .map(|(ns, pairs)| {
                        let pairs_json: Vec<_> = pairs
                            .iter()
                            .map(|(from, to, count)| {
                                serde_json::json!({
                                    "from": from,
                                    "to": to,
                                    "count": count,
                                })
                            })
                            .collect();
                        (ns.clone(), serde_json::json!(pairs_json))
                    })
                    .collect();

                serde_json::json!({
                    "conceptual_fan_out": {
                        "max": c.conceptual_fan_out.max,
                        "mean": c.conceptual_fan_out.mean,
                        "top": c.conceptual_fan_out.top.iter().map(|(id, count)| {
                            serde_json::json!({"id": id, "count": count})
                        }).collect::<Vec<_>>(),
                    },
                    "concept_scattering": scattering,
                    "cross_concept_edges": {
                        "total_tagged_edges": c.cross_concept_edges.total_tagged_edges,
                        "cross_concept_count": c.cross_concept_edges.cross_concept_count,
                        "cross_concept_ratio": c.cross_concept_edges.cross_concept_ratio,
                        "by_namespace": cross_concept_by_ns,
                    },
                })
            });

            let output = serde_json::json!({
                "files": {
                    "extracted": extracted_count,
                    "cached": cached_count,
                },
                "units": graph.units.len(),
                "edges": graph.edges.len(),
                "metrics": {
                    "dag_density": {
                        "value": bucketed.dag_density.value,
                        "bucket": bucketed.dag_density.bucket,
                    },
                    "fan_in": {
                        "max": {
                            "value": bucketed.fan_in.max.value as usize,
                            "bucket": bucketed.fan_in.max.bucket,
                        },
                        "mean": {
                            "value": bucketed.fan_in.mean.value,
                            "bucket": bucketed.fan_in.mean.bucket,
                        },
                    },
                    "fan_out": {
                        "max": {
                            "value": bucketed.fan_out.max.value as usize,
                            "bucket": bucketed.fan_out.max.bucket,
                        },
                        "mean": {
                            "value": bucketed.fan_out.mean.value,
                            "bucket": bucketed.fan_out.mean.bucket,
                        },
                    },
                    "complexity": {
                        "size": {
                            "max": complexity.size.max,
                            "mean": complexity.size.mean,
                            "p90": complexity.size.p90,
                        },
                        "params": {
                            "max": complexity.params.max,
                            "mean": complexity.params.mean,
                        },
                        "cyclomatic": {
                            "max": complexity.cyclomatic.max,
                            "mean": complexity.cyclomatic.mean,
                            "p90": complexity.cyclomatic.p90,
                        },
                    },
                    "impl": {
                        "methods_per_impl": {
                            "max": impl_metrics.methods_per_impl.max,
                            "mean": impl_metrics.methods_per_impl.mean,
                            "p90": impl_metrics.methods_per_impl.p90,
                        },
                        "traits_per_type": {
                            "max": impl_metrics.traits_per_type.max,
                            "mean": impl_metrics.traits_per_type.mean,
                        },
                        "lcom": {
                            "max": impl_metrics.lcom.max,
                            "mean": impl_metrics.lcom.mean,
                        },
                    },
                    "semantic_tags": {
                        "total_units": tag_metrics.total_units,
                        "tagged_units": tag_metrics.tagged_units,
                        "coverage": tag_metrics.tag_coverage,
                        "by_namespace": namespace_distribution,
                        "namespace_values": namespace_values,
                        "conceptual": conceptual_json,
                    }
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn handle_ls(path: &Path, kind_filter: Option<String>, format: OutputFormat) -> Result<()> {
    let store = CacheStore::open(path)?;
    let walker = SourceWalker::new(store.root());
    let semantic_tags = store.load_tags_with_staged()?;

    let kind_filter = kind_filter.map(|k| parse_unit_kind(&k)).transpose()?;

    let mut all_units: Vec<(Unit, Vec<String>)> = Vec::new();

    for file_path in walker.walk() {
        if let Ok(Some(entry)) = store.load_entry(&file_path) {
            for unit in entry.units {
                if let Some(ref filter) = kind_filter {
                    if &unit.kind != filter {
                        continue;
                    }
                }
                let tags = semantic_tags.get_tags(&unit.id).to_vec();
                all_units.push((unit, tags));
            }
        }
    }

    match format {
        OutputFormat::Text => {
            if all_units.is_empty() {
                println!("No symbols found. Run 'mdlr check --save' first.");
                return Ok(());
            }

            println!("{:<40} {:<10} {:<30} {:>6}-{:<6} {}", "ID", "Kind", "File", "Start", "End", "Tags");
            println!("{}", "-".repeat(120));
            for (unit, tags) in &all_units {
                let kind_str = format!("{:?}", unit.kind);
                let file_str = unit.file.display().to_string();
                let tags_str = if tags.is_empty() {
                    String::new()
                } else {
                    tags.join(", ")
                };
                println!(
                    "{:<40} {:<10} {:<30} {:>6}-{:<6} {}",
                    truncate(&unit.id, 40),
                    kind_str,
                    truncate(&file_str, 30),
                    unit.span.start_line,
                    unit.span.end_line,
                    tags_str
                );
            }
            println!();
            println!("Total: {} symbols", all_units.len());
        }
        OutputFormat::Json => {
            let output: Vec<_> = all_units
                .into_iter()
                .map(|(unit, tags)| {
                    serde_json::json!({
                        "id": unit.id,
                        "kind": format!("{:?}", unit.kind),
                        "file": unit.file,
                        "span": {
                            "start_line": unit.span.start_line,
                            "end_line": unit.span.end_line,
                        },
                        "tags": tags,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn handle_get(symbol: &str, format: OutputFormat) -> Result<()> {
    let store = CacheStore::open(Path::new("."))?;
    let walker = SourceWalker::new(store.root());
    let semantic_tags = store.load_tags_with_staged()?;

    // Find the unit
    let mut found_unit: Option<Unit> = None;
    for file_path in walker.walk() {
        if let Ok(Some(entry)) = store.load_entry(&file_path) {
            for unit in entry.units {
                if unit.id == symbol {
                    found_unit = Some(unit);
                    break;
                }
            }
        }
        if found_unit.is_some() {
            break;
        }
    }

    let unit = match found_unit {
        Some(u) => u,
        None => bail!("Symbol '{}' not found. Run 'mdlr ls' to see available symbols.", symbol),
    };

    // Read the source file and extract the span
    let source_path = store.root().join(&unit.file);
    let source = fs::read_to_string(&source_path)?;
    let lines: Vec<&str> = source.lines().collect();

    let start_idx = unit.span.start_line.saturating_sub(1);
    let end_idx = unit.span.end_line.min(lines.len());
    let content: String = lines[start_idx..end_idx].join("\n");

    let tags = semantic_tags.get_tags(&unit.id).to_vec();

    match format {
        OutputFormat::Text => {
            println!("Symbol: {}", unit.id);
            println!("Kind: {:?}", unit.kind);
            println!("File: {}:{}-{}", unit.file.display(), unit.span.start_line, unit.span.end_line);
            if !tags.is_empty() {
                println!("Tags: {}", tags.join(", "));
            }
            println!();
            println!("{}", content);
        }
        OutputFormat::Json => {
            let output = serde_json::json!({
                "id": unit.id,
                "kind": format!("{:?}", unit.kind),
                "file": unit.file,
                "span": {
                    "start_line": unit.span.start_line,
                    "end_line": unit.span.end_line,
                },
                "tags": tags,
                "content": content,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
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

    // List all tags (with staged changes overlaid)
    if list {
        let semantic_tags = store.load_tags_with_staged()?;
        let has_staged = store.has_staged_tags();

        match format {
            OutputFormat::Text => {
                if semantic_tags.tags.is_empty() {
                    println!("No semantic tags defined.");
                    return Ok(());
                }
                println!("{:<40} {}", "Symbol", "Tags");
                println!("{}", "-".repeat(80));
                let mut entries: Vec<_> = semantic_tags.tags.iter().collect();
                entries.sort_by_key(|(k, _)| k.as_str());
                for (unit_id, tags) in entries {
                    println!("{:<40} {}", truncate(unit_id, 40), tags.join(", "));
                }
                if has_staged {
                    println!();
                    println!("(staged changes pending - use 'mdlr check --save' to commit)");
                }
            }
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&semantic_tags.tags)?);
            }
        }
        return Ok(());
    }

    // Require symbol for add/remove/clear operations
    let symbol = match symbol {
        Some(s) => s,
        None => bail!("Symbol ID is required. Use 'mdlr tag --list' to see all tags, or specify a symbol."),
    };

    // Verify symbol exists
    let walker = SourceWalker::new(store.root());
    let mut symbol_exists = false;
    for file_path in walker.walk() {
        if let Ok(Some(entry)) = store.load_entry(&file_path) {
            if entry.units.iter().any(|u| u.id == symbol) {
                symbol_exists = true;
                break;
            }
        }
    }
    if !symbol_exists {
        bail!("Symbol '{}' not found. Run 'mdlr ls' to see available symbols.", symbol);
    }

    // Load staged tags for modifications
    let mut staged = store.load_staged_tags()?;

    // Clear tags
    if clear {
        staged.stage_clear(&symbol);
        store.save_staged_tags(&staged)?;
        println!("Staged: clear all tags from '{}' (use 'mdlr check --save' to commit)", symbol);
        return Ok(());
    }

    // Remove a tag
    if let Some(ref tag) = remove {
        staged.stage_remove(&symbol, tag);
        store.save_staged_tags(&staged)?;
        println!("Staged: remove tag '{}' from '{}' (use 'mdlr check --save' to commit)", tag, symbol);
        return Ok(());
    }

    // Add tags
    if !add.is_empty() {
        for tag in &add {
            staged.stage_add(&symbol, tag)?;
        }
        store.save_staged_tags(&staged)?;
        println!("Staged: add {} tag(s) to '{}' (use 'mdlr check --save' to commit)", add.len(), symbol);
        return Ok(());
    }

    // Show current tags for symbol (with staged changes)
    let semantic_tags = store.load_tags_with_staged()?;
    let tags = semantic_tags.get_tags(&symbol);
    match format {
        OutputFormat::Text => {
            if tags.is_empty() {
                println!("No tags on '{}'", symbol);
            } else {
                println!("Tags on '{}': {}", symbol, tags.join(", "));
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&tags)?);
        }
    }

    Ok(())
}

fn parse_unit_kind(s: &str) -> Result<UnitKind> {
    match s.to_lowercase().as_str() {
        "function" | "fn" => Ok(UnitKind::Function),
        "struct" => Ok(UnitKind::Struct),
        "module" | "mod" => Ok(UnitKind::Module),
        "trait" => Ok(UnitKind::Trait),
        "impl" => Ok(UnitKind::Impl),
        _ => bail!("Unknown unit kind '{}'. Valid kinds: function, struct, module, trait, impl", s),
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

fn build_graph(units: Vec<Unit>, resolution_ctx: Option<&ResolutionContext>) -> Graph {
    let mut graph = Graph::new();

    // Build multiple resolution indexes for call matching
    // 1. Exact match: full ID -> full ID
    // 2. Short name: "function_name" -> full ID (may have conflicts)
    // 3. With impl: "impl Foo::method" -> full ID
    // 4. Module path: "module::function" -> full ID

    let unit_ids: HashSet<_> = units.iter().map(|u| u.id.clone()).collect();

    // Map from various name forms to full IDs
    // We store Vec<String> to handle ambiguous names (multiple functions with same short name)
    let mut name_to_ids: HashMap<String, Vec<String>> = HashMap::new();

    for unit in &units {
        // Add the full ID (for exact matches)
        name_to_ids
            .entry(unit.id.clone())
            .or_default()
            .push(unit.id.clone());

        // Extract the local part (after the first "::")
        // For crate-based IDs: "my_crate::module::func" -> "module::func"
        // For file-based IDs: "src/main.rs::handle_check" -> "handle_check"
        if let Some(idx) = unit.id.find("::") {
            let local = &unit.id[idx + 2..];

            // Add local name
            name_to_ids
                .entry(local.to_string())
                .or_default()
                .push(unit.id.clone());

            // For methods in impl blocks, also index by just the method name
            // e.g., "impl Foo::new" -> also index as "new" and "Foo::new"
            if local.contains("::impl ") || local.starts_with("impl ") {
                // Find the impl part and extract method name
                let impl_start = local.find("impl ").unwrap_or(0);
                let impl_part = &local[impl_start..];

                // "impl Foo::method" -> extract "method" and "Foo::method"
                if let Some(method_idx) = impl_part.rfind("::") {
                    let method_name = &impl_part[method_idx + 2..];
                    name_to_ids
                        .entry(method_name.to_string())
                        .or_default()
                        .push(unit.id.clone());

                    // Also add "Type::method" form (without "impl ")
                    // "impl Foo::method" -> "Foo::method"
                    let type_and_method = &impl_part[5..]; // Skip "impl "
                    name_to_ids
                        .entry(type_and_method.to_string())
                        .or_default()
                        .push(unit.id.clone());
                }
            }

            // For crate-based IDs, also index by the short name (last segment)
            // e.g., "my_crate::module::func" -> "func"
            if let Some(last_idx) = local.rfind("::") {
                let short_name = &local[last_idx + 2..];
                if !short_name.is_empty() && !short_name.starts_with("impl ") {
                    name_to_ids
                        .entry(short_name.to_string())
                        .or_default()
                        .push(unit.id.clone());
                }
            }
        }
    }

    // Resolve calls and create edges
    for unit in &units {
        let caller_file = unit.file.to_string_lossy();

        for call in &unit.calls {
            // First check if the call is already a fully resolved crate path that matches a unit ID
            // This handles calls that were resolved during extraction
            if unit_ids.contains(call) {
                if call != &unit.id {
                    graph.add_edge(Edge {
                        from: unit.id.clone(),
                        to: call.clone(),
                        kind: EdgeKind::Calls,
                    });
                }
                continue;
            }

            // Try resolution context first (if available), then fall back to heuristic resolution
            let resolved = resolution_ctx
                .and_then(|ctx| resolve_call_with_context(call, &unit.file, ctx, &unit_ids))
                .or_else(|| resolve_call(call, &caller_file, &unit_ids, &name_to_ids));

            if let Some(target_id) = resolved {
                // Don't create self-loops
                if target_id != unit.id {
                    graph.add_edge(Edge {
                        from: unit.id.clone(),
                        to: target_id,
                        kind: EdgeKind::Calls,
                    });
                }
            }
        }
    }

    for unit in units {
        graph.add_unit(unit);
    }

    graph
}

/// Resolve a call expression to a fully qualified unit ID
fn resolve_call(
    call: &str,
    caller_file: &str,
    unit_ids: &HashSet<String>,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    // 1. Try exact match first (for fully qualified calls)
    if unit_ids.contains(call) {
        return Some(call.to_string());
    }

    // 2. Try prefixing with caller's file path (same-file calls)
    let same_file_id = format!("{}::{}", caller_file, call);
    if unit_ids.contains(&same_file_id) {
        return Some(same_file_id);
    }

    // 3. Look up in name index
    if let Some(candidates) = name_to_ids.get(call) {
        // If only one match, use it
        if candidates.len() == 1 {
            return Some(candidates[0].clone());
        }

        // Multiple matches - prefer same file
        for candidate in candidates {
            if candidate.starts_with(caller_file) {
                return Some(candidate.clone());
            }
        }

        // If still ambiguous, take the first one (arbitrary but deterministic)
        // In practice, if there are multiple `new()` functions, we can't know
        // which one is being called without type information
        return Some(candidates[0].clone());
    }

    // 4. Handle method calls like "self.field" or "obj.method"
    // These typically won't resolve to our units since we don't track
    // what type "obj" is, but we try anyway
    if call.contains('.') {
        // Try just the method name
        if let Some(method) = call.rsplit('.').next() {
            if let Some(candidates) = name_to_ids.get(method) {
                if candidates.len() == 1 {
                    return Some(candidates[0].clone());
                }
                // Ambiguous method call - prefer same file
                for candidate in candidates {
                    if candidate.starts_with(caller_file) {
                        return Some(candidate.clone());
                    }
                }
            }
        }
    }

    // 5. Handle path-style calls like "module::function" or "Type::method"
    if call.contains("::") {
        // Already tried exact match and name index lookup above
        // Try removing the first component (crate/module name)
        if let Some(idx) = call.find("::") {
            let without_prefix = &call[idx + 2..];
            if let Some(candidates) = name_to_ids.get(without_prefix) {
                if candidates.len() == 1 {
                    return Some(candidates[0].clone());
                }
            }
        }
    }

    None
}

/// Resolve a call using the semantic resolution context.
///
/// This uses Cargo workspace information, module graphs, and use statements
/// to provide more accurate resolution than heuristic matching.
fn resolve_call_with_context(
    call: &str,
    caller_file: &Path,
    ctx: &ResolutionContext,
    unit_ids: &HashSet<String>,
) -> Option<String> {
    // First, try to resolve using the semantic context
    let resolved_path = ctx.resolve_call(call, caller_file)?;

    // Now try to map the resolved path back to a unit ID
    // The resolved path looks like "crate_name::module::item"
    // but our unit IDs look like "src/file.rs::item"

    // Strategy 1: Check if resolved path matches any unit ID directly
    if unit_ids.contains(&resolved_path) {
        return Some(resolved_path);
    }

    // Strategy 2: Try to find the unit by matching the item name
    // Extract the item name from the resolved path
    let item_name = resolved_path.rsplit("::").next()?;

    // Look for units that end with this item name
    for unit_id in unit_ids {
        // Check if the unit ID ends with "::item_name"
        if unit_id.ends_with(&format!("::{}", item_name)) {
            // If there's a file path in the resolved path, try to match it
            // For now, just return the first match (could be improved with better heuristics)
            return Some(unit_id.clone());
        }
    }

    // Strategy 3: For cross-crate resolution, find the matching crate's files
    // The resolution context knows which crate each file belongs to
    if let Some((resolved_crate, _)) = ctx.file_to_module(caller_file) {
        // If the resolved path starts with a different crate, find that crate's units
        let path_parts: Vec<&str> = resolved_path.split("::").collect();
        if !path_parts.is_empty() {
            let target_crate = path_parts[0];
            if target_crate != resolved_crate && target_crate != "crate" && target_crate != "std" {
                // Look for units in that crate
                for unit_id in unit_ids {
                    // Extract crate info from unit ID's file path
                    // This is a heuristic - check if the file path contains the crate name
                    if unit_id.contains(&format!("{}/", target_crate.replace('_', "-")))
                        || unit_id.contains(&format!("{}/", target_crate))
                    {
                        if unit_id.ends_with(&format!("::{}", item_name)) {
                            return Some(unit_id.clone());
                        }
                    }
                }
            }
        }
    }

    None
}

fn extract_file(
    abs_path: &Path,
    extractor: &dyn Extractor,
    resolution_ctx: Option<&ResolutionContext>,
) -> Result<Vec<Unit>> {
    let source = fs::read_to_string(abs_path)?;
    extractor.extract(&source, abs_path, resolution_ctx)
}
