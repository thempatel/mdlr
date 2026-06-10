use anyhow::{Context, Result};
use std::env;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use crate::cache::{CacheStore, FileCacheEntry};

/// A language mdlr can extract Units from, identified by the file extensions
/// it owns. One `const` per language (see [`LANGUAGES`]).
pub struct Language {
    pub extensions: &'static [&'static str],
}

const RUST: Language = Language { extensions: &["rs"] };
const TYPESCRIPT: Language =
    Language { extensions: &["ts", "tsx", "js", "jsx"] };
const GO: Language = Language { extensions: &["go"] };
const PYTHON: Language = Language { extensions: &["py", "pyi"] };

/// The single source of truth for which file extensions mdlr can extract.
/// Go is listed here too even though it's a subprocess binary (not a linked
/// crate), so the registry stays uniform across languages.
pub const LANGUAGES: &[&Language] = &[&RUST, &TYPESCRIPT, &GO, &PYTHON];

/// Whether `ext` (without leading dot) belongs to a supported language.
pub fn is_source_extension(ext: &str) -> bool {
    LANGUAGES.iter().any(|l| l.extensions.contains(&ext))
}

/// Whether `path` is a source file mdlr can extract from, by its extension.
pub fn is_source_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(is_source_extension)
}

// SAFETY justification for `unsafe { env::set_var(...) }` below: the rust
// extractor's `MDLR_QUIET_DIAGNOSTICS` is read at extractor entry, before any
// background thread the rust-analyzer libs spawn could observe a partial
// value. We only set it; we never unset it concurrently.

/// Run an in-process extractor and convert errors/panics into a warning.
///
/// Returns `true` if the extractor completed cleanly, `false` if it errored or
/// panicked (in which case a warning is printed). Wrapping in `catch_unwind`
/// keeps a panic in any single extractor (rust-analyzer in particular) from
/// terminating the whole `mdlr` invocation.
fn run_extractor<F>(language: &str, f: F) -> bool
where
    F: FnOnce() -> Result<()>,
{
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    match result {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            eprintln!(
                "Warning: {language} extraction had errors (results may be partial): {e:#}"
            );
            false
        }
        Err(_) => {
            eprintln!(
                "Warning: {language} extraction panicked (results may be partial)"
            );
            false
        }
    }
}

/// Run the in-process Rust extractor against the workspace at `store.root()`.
///
/// Only runs if a `Cargo.toml` exists at the workspace root.
#[tracing::instrument(name = "extract", skip_all)]
pub fn extract_rust(store: &CacheStore, generation_id: u64) -> Result<bool> {
    let workspace_root = store.root();

    let manifest_path = workspace_root.join("Cargo.toml");
    if !manifest_path.exists() {
        return Ok(true);
    }

    // The rust extractor's diagnostics path is gated on this env var; preserve
    // the prior behavior where the orchestrator always set it before invoking.
    unsafe {
        env::set_var("MDLR_QUIET_DIAGNOSTICS", "1");
    }

    let cache_dir = store.cache_dir().to_path_buf();
    let workspace_root = workspace_root.to_path_buf();
    let success = run_extractor("Rust", || {
        mdlr_extract_rust::extract(
            &manifest_path,
            &cache_dir,
            Some(generation_id),
            &[],
            &workspace_root,
        )
    });

    Ok(success)
}

/// Detect whether the project has TypeScript/JavaScript files.
pub fn has_ts_files(root: &Path) -> bool {
    if root.join("tsconfig.json").exists()
        || root.join("package.json").exists()
    {
        return true;
    }
    let walker =
        ignore::WalkBuilder::new(root).hidden(true).max_depth(Some(3)).build();
    for entry in walker.flatten() {
        if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
            if TYPESCRIPT.extensions.contains(&ext) {
                return true;
            }
        }
    }
    false
}

/// Run the in-process TS/JS extractor against the workspace at `store.root()`.
#[tracing::instrument(name = "extract_ts", skip_all)]
pub fn extract_ts(store: &CacheStore, generation_id: u64) -> Result<bool> {
    let workspace_root = store.root();
    if !has_ts_files(workspace_root) {
        return Ok(true);
    }

    let cache_dir = store.cache_dir().to_path_buf();
    let workspace_root = workspace_root.to_path_buf();
    let success = run_extractor("TS", || {
        mdlr_extract_ts::extract(
            &workspace_root,
            &cache_dir,
            Some(generation_id),
        )
    });

    Ok(success)
}

/// Find the `mdlr-extract-go` binary, checking next to our own binary first.
fn find_extract_go_binary() -> Option<PathBuf> {
    if let Ok(current_exe) = env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let sibling = dir.join("mdlr-extract-go");
            if sibling.exists() {
                return Some(sibling);
            }
        }
    }
    if let Ok(output) =
        std::process::Command::new("which").arg("mdlr-extract-go").output()
    {
        if output.status.success() {
            let path =
                String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

/// Shell out to `mdlr-extract-go` to extract units from Go files.
///
/// Only runs if a `go.mod` exists at the workspace root.
#[tracing::instrument(name = "extract_go", skip_all)]
pub fn extract_go(store: &CacheStore, generation_id: u64) -> Result<bool> {
    let extract_bin = match find_extract_go_binary() {
        Some(bin) => bin,
        None => return Ok(true),
    };

    let workspace_root = store.root();
    if !workspace_root.join("go.mod").exists() {
        return Ok(true);
    }

    let status = std::process::Command::new(&extract_bin)
        .arg("--root")
        .arg(workspace_root)
        .arg("--output")
        .arg(store.cache_dir())
        .arg("--generation-id")
        .arg(generation_id.to_string())
        .current_dir(workspace_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("Failed to run mdlr-extract-go")?;

    Ok(status.success())
}

/// Detect whether the project has Python files.
pub fn has_python_project(root: &Path) -> bool {
    if root.join("pyproject.toml").exists()
        || root.join("setup.py").exists()
        || root.join("setup.cfg").exists()
    {
        return true;
    }
    let walker =
        ignore::WalkBuilder::new(root).hidden(true).max_depth(Some(3)).build();
    for entry in walker.flatten() {
        if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
            if PYTHON.extensions.contains(&ext) {
                return true;
            }
        }
    }
    false
}

/// Run the in-process Python extractor against the workspace at `store.root()`.
#[tracing::instrument(name = "extract_py", skip_all)]
pub fn extract_py(store: &CacheStore, generation_id: u64) -> Result<bool> {
    let workspace_root = store.root();
    if !has_python_project(workspace_root) {
        return Ok(true);
    }

    let cache_dir = store.cache_dir().to_path_buf();
    let workspace_root = workspace_root.to_path_buf();
    let success = run_extractor("Python", || {
        mdlr_extract_py::extract(
            &workspace_root,
            &cache_dir,
            Some(generation_id),
        )
    });

    Ok(success)
}

/// Load both cache kinds (FileCacheEntry JSON + .tokens binaries) from a
/// cache directory.
pub fn load_cache_dir(
    dir: &Path,
) -> Result<(Vec<FileCacheEntry>, Vec<mdlr_cpd::FileTokens>)> {
    let mut entries = Vec::new();
    load_entries_from_dir(dir, &mut entries)?;
    let mut tokens = Vec::new();
    load_tokens_from_dir(dir, &mut tokens)?;
    Ok((entries, tokens))
}

/// Recursively load FileCacheEntry JSON files from a directory.
#[tracing::instrument(name = "load_cache", skip_all)]
pub fn load_entries_from_dir(
    dir: &Path,
    entries: &mut Vec<FileCacheEntry>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        if path.is_dir() {
            load_entries_from_dir(&path, entries)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let content =
                std::fs::read_to_string(&path).with_context(|| {
                    format!("Failed to read {}", path.display())
                })?;
            let entry: FileCacheEntry = serde_json::from_str(&content)
                .with_context(|| {
                    format!("Failed to parse {}", path.display())
                })?;
            entries.push(entry);
        }
    }
    Ok(())
}

/// Recursively load .tokens binary files from a directory.
#[tracing::instrument(name = "load_tokens", skip_all)]
pub fn load_tokens_from_dir(
    dir: &Path,
    tokens: &mut Vec<mdlr_cpd::FileTokens>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        if path.is_dir() {
            load_tokens_from_dir(&path, tokens)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("tokens") {
            let data = std::fs::read(&path).with_context(|| {
                format!("Failed to read {}", path.display())
            })?;
            match mdlr_cpd::binary::deserialize(&data) {
                Ok(file_tokens) => tokens.push(file_tokens),
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to parse token cache {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    }
    Ok(())
}
