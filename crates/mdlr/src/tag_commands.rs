//! Tag-related command handlers.

use crate::cache::CacheStore;
use crate::cli::OutputFormat;
use crate::walk::SourceWalker;
use anyhow::{Result, bail};

/// List all tags
pub fn handle_tag_list(
    store: &CacheStore,
    format: OutputFormat,
) -> Result<()> {
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
                println!(
                    "(staged changes pending - use 'mdlr check --save' to commit)"
                );
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&semantic_tags.tags)?);
        }
    }
    Ok(())
}

/// Verify a symbol exists in the codebase
pub fn verify_symbol_exists(store: &CacheStore, symbol: &str) -> Result<()> {
    let walker = SourceWalker::new(store.root());
    for file_path in walker.walk() {
        if let Ok(Some(entry)) = store.load_entry(&file_path) {
            if entry.units.iter().any(|u| u.id == symbol) {
                return Ok(());
            }
        }
    }
    bail!(
        "Symbol '{}' not found. Run 'mdlr ls' to see available symbols.",
        symbol
    );
}

/// Clear all tags from a symbol
pub fn handle_tag_clear(store: &CacheStore, symbol: &str) -> Result<()> {
    let mut staged = store.load_staged_tags()?;
    staged.stage_clear(symbol);
    store.save_staged_tags(&staged)?;
    println!(
        "Staged: clear all tags from '{}' (use 'mdlr check --save' to commit)",
        symbol
    );
    Ok(())
}

/// Remove a tag from a symbol
pub fn handle_tag_remove(
    store: &CacheStore,
    symbol: &str,
    tag: &str,
) -> Result<()> {
    let mut staged = store.load_staged_tags()?;
    staged.stage_remove(symbol, tag);
    store.save_staged_tags(&staged)?;
    println!(
        "Staged: remove tag '{}' from '{}' (use 'mdlr check --save' to commit)",
        tag, symbol
    );
    Ok(())
}

/// Add tags to a symbol
pub fn handle_tag_add(
    store: &CacheStore,
    symbol: &str,
    tags: &[String],
) -> Result<()> {
    let mut staged = store.load_staged_tags()?;
    for tag in tags {
        staged.stage_add(symbol, tag)?;
    }
    store.save_staged_tags(&staged)?;
    println!(
        "Staged: add {} tag(s) to '{}' (use 'mdlr check --save' to commit)",
        tags.len(),
        symbol
    );
    Ok(())
}

/// Show tags on a symbol
pub fn handle_tag_show(
    store: &CacheStore,
    symbol: &str,
    format: OutputFormat,
) -> Result<()> {
    let semantic_tags = store.load_tags_with_staged()?;
    let tags = semantic_tags.get_tags(symbol);
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

/// Truncate a string to max_len characters
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
