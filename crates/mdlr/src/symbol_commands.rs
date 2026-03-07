//! Symbol listing and retrieval command handlers.

use anyhow::{Result, bail};
use std::fs;
use std::path::Path;

use crate::cache::CacheStore;
use crate::cli::OutputFormat;
use crate::walk::SourceWalker;
use mdlr_core::{Unit, UnitKind};

/// Collect all units, optionally filtered by kind.
fn collect_units(
    store: &CacheStore,
    kind_filter: Option<UnitKind>,
) -> Result<Vec<Unit>> {
    let walker = SourceWalker::new(store.root());
    let mut all_units = Vec::new();

    for file_path in walker.walk() {
        if let Ok(Some(entry)) = store.load_entry(&file_path) {
            for unit in entry.units {
                if let Some(ref filter) = kind_filter {
                    if &unit.kind != filter {
                        continue;
                    }
                }
                all_units.push(unit);
            }
        }
    }

    Ok(all_units)
}

/// Handle the 'ls' command to list symbols
pub fn handle_ls(
    path: &Path,
    kind_filter: Option<String>,
    format: OutputFormat,
) -> Result<()> {
    let store = CacheStore::open(path)?;
    let kind_filter = kind_filter.map(|k| parse_unit_kind(&k)).transpose()?;
    let all_units = collect_units(&store, kind_filter)?;

    match format {
        OutputFormat::Text => print_ls_text(&all_units),
        OutputFormat::Json => print_ls_json(&all_units)?,
    }

    Ok(())
}

fn print_ls_text(all_units: &[Unit]) {
    if all_units.is_empty() {
        println!("No symbols found. Run 'mdlr check' first.");
        return;
    }

    println!(
        "{:<40} {:<10} {:<30} {:>6}-{:<6}",
        "ID", "Kind", "File", "Start", "End"
    );
    println!("{}", "-".repeat(100));
    for unit in all_units {
        let kind_str = format!("{:?}", unit.kind);
        let file_str = unit.file.display().to_string();
        println!(
            "{:<40} {:<10} {:<30} {:>6}-{:<6}",
            truncate(&unit.id, 40),
            kind_str,
            truncate(&file_str, 30),
            unit.span.start_line,
            unit.span.end_line,
        );
    }
    println!();
    println!("Total: {} symbols", all_units.len());
}

fn print_ls_json(all_units: &[Unit]) -> Result<()> {
    let output: Vec<_> = all_units
        .iter()
        .map(|unit| {
            serde_json::json!({
                "id": unit.id,
                "kind": format!("{:?}", unit.kind),
                "file": unit.file,
                "span": {
                    "start_line": unit.span.start_line,
                    "end_line": unit.span.end_line,
                },
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Find a unit by symbol ID in the cache.
fn find_unit(store: &CacheStore, symbol: &str) -> Result<Unit> {
    let walker = SourceWalker::new(store.root());
    for file_path in walker.walk() {
        if let Ok(Some(entry)) = store.load_entry(&file_path) {
            for unit in entry.units {
                if unit.id == symbol {
                    return Ok(unit);
                }
            }
        }
    }
    bail!(
        "Symbol '{}' not found. Run 'mdlr ls' to see available symbols.",
        symbol
    )
}

/// Handle the 'get' command to retrieve a symbol
pub fn handle_get(symbol: &str, format: OutputFormat) -> Result<()> {
    let store = CacheStore::open(Path::new("."))?;
    let unit = find_unit(&store, symbol)?;

    // Read the source file and extract the span
    let source_path = store.root().join(&unit.file);
    let source = fs::read_to_string(&source_path)?;
    let lines: Vec<&str> = source.lines().collect();

    let start_idx = unit.span.start_line.saturating_sub(1);
    let end_idx = unit.span.end_line.min(lines.len());
    let content: String = lines[start_idx..end_idx].join("\n");

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
