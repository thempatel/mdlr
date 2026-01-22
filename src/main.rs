use anyhow::Result;
use clap::Parser;
use mdlr::cache::{get_file_metadata, now_timestamp, CacheStore, FileCacheEntry, ProjectIndex};
use mdlr::cli::{Cli, Command, OutputFormat};
use mdlr::config;
use mdlr::extract::{extractor_for_path, Extractor};
use mdlr::graph::{Edge, EdgeKind, Graph, Unit};
use mdlr::metrics::{BucketedMetrics, MetricsDisplay};
use mdlr::walk::SourceWalker;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Todo { path, all, format } => handle_todo(&path, all, format),
        Command::Analyze { path, force, format } => handle_analyze(&path, force, format),
        Command::Export { path, format } => handle_export(&path, format),
    }
}

fn handle_todo(path: &Path, all: bool, format: OutputFormat) -> Result<()> {
    let store = CacheStore::open(path)?;
    let index = store.load_index()?;
    let walker = SourceWalker::new(store.root());

    let mut new_files = Vec::new();
    let mut changed_files = Vec::new();
    let mut untagged_files = Vec::new();

    for file_path in walker.walk() {
        let relative = file_path
            .strip_prefix(store.root())
            .unwrap_or(&file_path)
            .to_path_buf();

        let current_meta = get_file_metadata(&file_path)?;

        match index.files.get(&relative) {
            None => {
                new_files.push(relative);
            }
            Some(cached_meta) => {
                if cached_meta.mtime != current_meta.mtime || cached_meta.size != current_meta.size
                {
                    changed_files.push(relative);
                } else if all {
                    if let Ok(Some(entry)) = store.load_entry(&file_path) {
                        if entry.units.iter().any(|u| u.tags.is_empty()) {
                            untagged_files.push(relative);
                        }
                    }
                }
            }
        }
    }

    match format {
        OutputFormat::Text => {
            let has_work = !new_files.is_empty() || !changed_files.is_empty();
            let has_untagged = !untagged_files.is_empty();

            if !has_work && !has_untagged {
                println!("All files are up to date.");
                return Ok(());
            }

            if !new_files.is_empty() {
                println!("New files ({}):", new_files.len());
                for f in &new_files {
                    println!("  {}", f.display());
                }
                println!();
            }

            if !changed_files.is_empty() {
                println!("Changed files ({}):", changed_files.len());
                for f in &changed_files {
                    println!("  {}", f.display());
                }
                println!();
            }

            if all && !untagged_files.is_empty() {
                println!("Files with untagged units ({}):", untagged_files.len());
                for f in &untagged_files {
                    println!("  {}", f.display());
                }
                println!();
            }

            let total = new_files.len() + changed_files.len();
            if total > 0 {
                println!("Run 'mdlr analyze' to update {} file(s).", total);
            }
        }
        OutputFormat::Json => {
            let output = serde_json::json!({
                "new": new_files,
                "changed": changed_files,
                "untagged": if all { untagged_files } else { vec![] },
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn handle_analyze(path: &Path, force: bool, format: OutputFormat) -> Result<()> {
    let store = CacheStore::open(path)?;
    let config = config::load()?;
    let walker = SourceWalker::new(store.root());

    let mut index = if force {
        ProjectIndex::default()
    } else {
        store.load_index()?
    };

    let mut all_units: Vec<Unit> = Vec::new();
    let mut extracted_count = 0;
    let mut cached_count = 0;

    for file_path in walker.walk() {
        let relative = file_path
            .strip_prefix(store.root())
            .unwrap_or(&file_path)
            .to_path_buf();

        let current_meta = get_file_metadata(&file_path)?;
        let is_stale = force
            || index
                .files
                .get(&relative)
                .map(|m| m.mtime != current_meta.mtime || m.size != current_meta.size)
                .unwrap_or(true);

        let units = if is_stale {
            if let Some(extractor) = extractor_for_path(&file_path) {
                match extract_file(&file_path, extractor.as_ref()) {
                    Ok(units) => {
                        let entry = FileCacheEntry {
                            source_path: relative.clone(),
                            mtime: current_meta.mtime,
                            size: current_meta.size,
                            units: units.clone(),
                            cached_at: now_timestamp(),
                        };
                        store.save_entry(&entry)?;
                        index.files.insert(relative, current_meta);
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
        } else {
            match store.load_entry(&file_path)? {
                Some(entry) => {
                    cached_count += 1;
                    entry.units
                }
                None => continue,
            }
        };

        all_units.extend(units);
    }

    index.last_scan = now_timestamp();
    store.save_index(&index)?;

    let graph = build_graph(all_units);
    let metrics = mdlr::metrics::compute(&graph);

    match format {
        OutputFormat::Text => {
            println!("Analysis complete");
            println!();
            println!(
                "Files: {} extracted, {} from cache",
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
        }
        OutputFormat::Json => {
            let bucketed = BucketedMetrics::from_metrics(&metrics, &config);
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
                    }
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn handle_export(path: &Path, format: OutputFormat) -> Result<()> {
    let store = CacheStore::open(path)?;
    let walker = SourceWalker::new(store.root());

    let mut all_units: Vec<Unit> = Vec::new();

    for file_path in walker.walk() {
        if let Ok(Some(entry)) = store.load_entry(&file_path) {
            all_units.extend(entry.units);
        }
    }

    let graph = build_graph(all_units);

    match format {
        OutputFormat::Json => {
            let json = mdlr::graph::serialize::to_json(&graph)?;
            println!("{}", json);
        }
        OutputFormat::Text => {
            println!("Graph");
            println!();
            println!("Units ({}):", graph.units.len());
            for unit in &graph.units {
                println!("  {} ({:?}) - {:?}", unit.id, unit.kind, unit.file);
            }
            println!();
            println!("Edges ({}):", graph.edges.len());
            for edge in &graph.edges {
                println!("  {} -> {} ({:?})", edge.from, edge.to, edge.kind);
            }
        }
    }

    Ok(())
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
