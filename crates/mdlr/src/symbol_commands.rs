//! Symbol listing and retrieval command handlers.

use anyhow::{Result, bail};
use std::fs;
use std::path::Path;

use crate::cache::CacheStore;
use crate::cli::OutputFormat;
use crate::walk::SourceWalker;
use mdlr_core::{Unit, UnitKind};

/// Handle the 'ls' command to list symbols
pub fn handle_ls(
    path: &Path,
    kind_filter: Option<String>,
    format: OutputFormat,
) -> Result<()> {
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
        OutputFormat::Text => print_ls_text(&all_units),
        OutputFormat::Json => print_ls_json(all_units)?,
    }

    Ok(())
}

fn print_ls_text(all_units: &[(Unit, Vec<String>)]) {
    if all_units.is_empty() {
        println!("No symbols found. Run 'mdlr check --save' first.");
        return;
    }

    println!(
        "{:<40} {:<10} {:<30} {:>6}-{:<6} {}",
        "ID", "Kind", "File", "Start", "End", "Tags"
    );
    println!("{}", "-".repeat(120));
    for (unit, tags) in all_units {
        let kind_str = format!("{:?}", unit.kind);
        let file_str = unit.file.display().to_string();
        let tags_str =
            if tags.is_empty() { String::new() } else { tags.join(", ") };
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

fn print_ls_json(all_units: Vec<(Unit, Vec<String>)>) -> Result<()> {
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
    Ok(())
}

/// Handle the 'get' command to retrieve a symbol
pub fn handle_get(symbol: &str, format: OutputFormat) -> Result<()> {
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
        None => bail!(
            "Symbol '{}' not found. Run 'mdlr ls' to see available symbols.",
            symbol
        ),
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
            println!(
                "File: {}:{}-{}",
                unit.file.display(),
                unit.span.start_line,
                unit.span.end_line
            );
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

/// Parse a unit kind string
pub fn parse_unit_kind(s: &str) -> Result<UnitKind> {
    match s.to_lowercase().as_str() {
        "function" | "fn" => Ok(UnitKind::Function),
        "method" => Ok(UnitKind::Method),
        "struct" => Ok(UnitKind::Struct),
        "module" | "mod" => Ok(UnitKind::Module),
        _ => bail!(
            "Unknown unit kind '{}'. Valid kinds: function, method, struct, module",
            s
        ),
    }
}

/// Truncate a string to max_len characters
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
