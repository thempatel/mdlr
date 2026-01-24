use anyhow::{bail, Result};
use clap::Parser;
use mdlr::cache::{get_file_metadata, now_timestamp, CacheStore, FileCacheEntry};
use mdlr::cli::{Cli, Command, OutputFormat};
use mdlr::config;
use mdlr::extract::{extractor_for_path, Extractor};
use mdlr::graph::{Edge, EdgeKind, Graph, Unit, UnitKind};
use mdlr::metrics::{BucketedMetrics, ComplexityMetrics, ImplMetrics, MetricsDisplay, TagMetrics};
use mdlr::walk::SourceWalker;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Check { path, save, format } => handle_check(path.as_deref(), save, format),
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

fn handle_check(filter_path: Option<&Path>, save: bool, format: OutputFormat) -> Result<()> {
    let cwd = env::current_dir()?;
    let store = CacheStore::find_or_create(&cwd)?;
    let config = config::load()?;
    let walker = SourceWalker::new(store.root());

    // Canonicalize filter path for comparison
    let filter_path = filter_path
        .map(|p| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                cwd.join(p)
            }
        })
        .map(|p| p.canonicalize().unwrap_or(p));

    let mut all_units: Vec<Unit> = Vec::new();
    let mut extracted_count = 0;
    let mut cached_count = 0;

    // Track entries to save if --save is used
    let mut entries_to_save: Vec<FileCacheEntry> = Vec::new();

    for file_path in walker.walk() {
        // Apply path filter if specified
        if let Some(ref filter) = filter_path {
            if filter.is_file() {
                // Filter is a file: only include this exact file
                if file_path != *filter {
                    continue;
                }
            } else {
                // Filter is a directory: only include files under it
                if !file_path.starts_with(filter) {
                    continue;
                }
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
                    match extract_file(&file_path, extractor.as_ref()) {
                        Ok(units) => {
                            if save {
                                entries_to_save.push(FileCacheEntry {
                                    source_path: relative,
                                    mtime: current_meta.mtime,
                                    size: current_meta.size,
                                    units: units.clone(),
                                    cached_at: now_timestamp(),
                                });
                            }
                            extracted_count += 1;
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
                match extract_file(&file_path, extractor.as_ref()) {
                    Ok(units) => {
                        if save {
                            entries_to_save.push(FileCacheEntry {
                                source_path: relative,
                                mtime: current_meta.mtime,
                                size: current_meta.size,
                                units: units.clone(),
                                cached_at: now_timestamp(),
                            });
                        }
                        extracted_count += 1;
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

        all_units.extend(units);
    }

    // Save entries and commit staged tags if --save flag was provided
    if save {
        for entry in entries_to_save {
            store.save_entry(&entry)?;
        }
        // Commit any staged tag changes
        store.commit_staged_tags()?;
    }

    let graph = build_graph(all_units);
    let metrics = mdlr::metrics::compute(&graph);
    let complexity = ComplexityMetrics::compute(&graph);
    let impl_metrics = ImplMetrics::compute(&graph);
    // Load tags with staged changes overlaid
    let semantic_tags = store.load_tags_with_staged()?;
    let has_staged = store.has_staged_tags();
    let tag_metrics = TagMetrics::compute(&graph, &semantic_tags);

    match format {
        OutputFormat::Text => {
            println!(
                "Files: {} extracted, {} cached",
                extracted_count, cached_count
            );
            println!(
                "Graph: {} units, {} edges",
                graph.units.len(),
                graph.edges.len()
            );
            println!();
            let display = MetricsDisplay::new(&metrics, &config);
            print!("{}", display);

            if complexity.has_functions() {
                println!();
                print!("{}", complexity);
            }

            if impl_metrics.has_impls() {
                println!();
                print!("{}", impl_metrics);
            }

            if tag_metrics.has_tags() {
                println!();
                print!("{}", tag_metrics);
            }

            if has_staged {
                println!();
                println!("(staged tag changes pending - use --save to commit)");
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

fn build_graph(units: Vec<Unit>) -> Graph {
    let mut graph = Graph::new();
    let unit_ids: HashSet<_> = units.iter().map(|u| u.id.clone()).collect();

    for unit in &units {
        for call in &unit.calls {
            if unit_ids.contains(call) {
                graph.add_edge(Edge {
                    from: unit.id.clone(),
                    to: call.clone(),
                    kind: EdgeKind::Calls,
                });
            }
        }
    }

    for unit in units {
        graph.add_unit(unit);
    }

    graph
}

fn extract_file(path: &Path, extractor: &dyn Extractor) -> Result<Vec<Unit>> {
    let source = fs::read_to_string(path)?;
    extractor.extract(&source, path)
}
