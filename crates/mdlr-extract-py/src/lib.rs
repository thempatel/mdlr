mod branches;
#[cfg(test)]
mod branches_test;
mod calls;
#[cfg(test)]
mod calls_test;
mod cognitive;
#[cfg(test)]
mod cognitive_test;
mod field_access;
#[cfg(test)]
mod field_access_test;
mod scopes;
#[cfg(test)]
mod scopes_test;
mod tokenizer;
mod visitor;

use anyhow::{Context, Result};
use mdlr_core::FileCacheEntry;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// Extract Python units from all source files under `root`,
/// writing per-file `FileCacheEntry` JSON and `.tokens` blobs into `cache_dir`.
pub fn extract(
    root: &Path,
    cache_dir: &Path,
    generation_id: Option<u64>,
) -> Result<()> {
    let root = root.canonicalize().with_context(|| {
        format!("Failed to resolve root path: {}", root.display())
    })?;

    let timestamp = generation_id.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });

    let files = collect_files(&root)?;

    files.par_iter().for_each(|file_path| {
        if let Err(e) = process_file(file_path, &root, cache_dir, timestamp) {
            eprintln!("Failed to process {}: {e:#}", file_path.display());
        }
    });

    Ok(())
}

/// Collect all Python files under root, respecting .gitignore and common excludes.
fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            // Hardcoded excludes for Python ecosystem dirs
            !matches!(
                name.as_ref(),
                "__pycache__"
                    | ".venv"
                    | "venv"
                    | ".tox"
                    | "build"
                    | "dist"
                    | ".eggs"
                    | "node_modules"
            )
        })
        .build();

    for entry in walker {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "py" | "pyi" => files.push(path.to_path_buf()),
            _ => {}
        }
    }

    Ok(files)
}

/// Parse and extract units from a single Python file, writing JSON output.
fn process_file(
    file_path: &Path,
    root: &Path,
    output_dir: &Path,
    timestamp: u64,
) -> Result<()> {
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read {}", file_path.display()))?;

    let parsed = ruff_python_parser::parse_module(&source);

    let parsed = match parsed {
        Ok(m) => m,
        Err(_) => {
            // Parse errors — skip file
            return Ok(());
        }
    };

    let rel_path = file_path
        .strip_prefix(root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .replace('\\', "/");

    let units = visitor::extract_units(parsed.suite(), &source, &rel_path);

    let entry = FileCacheEntry {
        source_path: PathBuf::from(&rel_path),
        units,
        cached_at: timestamp,
    };

    let mut output_file = output_dir.join(&rel_path);
    output_file.set_extension("json");

    if let Some(parent) = output_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let json = serde_json::to_string_pretty(&entry)?;
    std::fs::write(&output_file, json).with_context(|| {
        format!("Failed to write {}", output_file.display())
    })?;

    // Write token cache for CPD
    let file_tokens = tokenizer::tokenize_py(&source, &rel_path, timestamp);
    let token_bytes = mdlr_cpd::binary::serialize(&file_tokens);
    let mut token_file = output_dir.join(&rel_path);
    token_file.set_extension("tokens");
    if let Err(e) = std::fs::write(&token_file, token_bytes) {
        eprintln!("Failed to write tokens for {}: {e}", rel_path);
    }

    Ok(())
}
