mod branches;
mod calls;
mod cognitive;
mod field_access;
mod scopes;
mod tokenizer;
mod visitor;

use anyhow::{Context, Result};
use mdlr_core::FileCacheEntry;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use swc_common::{SourceMap, sync::Lrc};
use swc_ecma_parser::{EsSyntax, Syntax, TsSyntax};

/// Extract TS/JS units from all source files under `root`,
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

/// Collect all TS/JS files under root, respecting .gitignore.
fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    let walker = ignore::WalkBuilder::new(root)
        .hidden(true) // skip hidden files
        .build();

    for entry in walker {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "ts" | "tsx" | "js" | "jsx" => files.push(path.to_path_buf()),
            _ => {}
        }
    }

    Ok(files)
}

/// Determine SWC syntax config for a file extension.
fn syntax_for_ext(ext: &str) -> Syntax {
    match ext {
        "ts" => Syntax::Typescript(TsSyntax {
            tsx: false,
            decorators: true,
            ..Default::default()
        }),
        "tsx" => Syntax::Typescript(TsSyntax {
            tsx: true,
            decorators: true,
            ..Default::default()
        }),
        "jsx" => Syntax::Es(EsSyntax { jsx: true, ..Default::default() }),
        _ => Syntax::Es(EsSyntax::default()),
    }
}

/// Parse and extract units from a single file, writing JSON output.
fn process_file(
    file_path: &Path,
    root: &Path,
    output_dir: &Path,
    timestamp: u64,
) -> Result<()> {
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("js");
    let syntax = syntax_for_ext(ext);

    let sm: Lrc<SourceMap> = Default::default();
    let source_file = sm
        .load_file(file_path)
        .with_context(|| format!("Failed to read {}", file_path.display()))?;

    let module = swc_ecma_parser::parse_file_as_module(
        &source_file,
        syntax,
        swc_ecma_ast::EsVersion::latest(),
        None,
        &mut vec![],
    );

    let module = match module {
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

    let units = visitor::extract_units(&module, &rel_path, &sm);

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
    if let Ok(source_text) = std::fs::read_to_string(file_path) {
        let file_tokens =
            tokenizer::tokenize_ts(&source_text, &rel_path, timestamp);
        let token_bytes = mdlr_cpd::binary::serialize(&file_tokens);
        let mut token_file = output_dir.join(&rel_path);
        token_file.set_extension("tokens");
        if let Err(e) = std::fs::write(&token_file, token_bytes) {
            eprintln!("Failed to write tokens for {}: {e}", rel_path);
        }
    }

    Ok(())
}
