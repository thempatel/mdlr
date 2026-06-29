mod branches;
mod calls;
mod cognitive;
mod field_access;
mod path_util;
mod scopes;
mod test_crate;
mod tokenizer;
mod visitor;
mod walk;

use anyhow::{Context, Result};
use mdlr_core::{cache_file_path, FileCacheEntry};
use ra_ap_hir::{attach_db, Crate, Semantics};
use ra_ap_load_cargo::{
    load_workspace_at, LoadCargoConfig, ProcMacroServerChoice,
};
use ra_ap_project_model::CargoConfig;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Extract Rust units from all workspace members at `manifest_path`,
/// writing per-file `FileCacheEntry` JSON and `.tokens` blobs into `cache_dir`.
///
/// `packages` filters which workspace members to extract from; pass an empty
/// slice to extract from all local workspace members.
///
/// `cwd` is the directory used to distinguish workspace-local crates from
/// external dependencies — when `packages` is empty, only crates whose root
/// file lives under `cwd` are extracted. Pass the workspace root.
pub fn extract(
    manifest_path: &Path,
    cache_dir: &Path,
    generation_id: Option<u64>,
    packages: &[String],
    cwd: &Path,
) -> Result<()> {
    let manifest_path = manifest_path.canonicalize().with_context(|| {
        format!("Failed to resolve manifest path: {}", manifest_path.display())
    })?;

    let workspace_dir = manifest_path
        .parent()
        .context("manifest path has no parent directory")?;

    let cargo_config =
        CargoConfig { sysroot: None, no_deps: true, ..CargoConfig::default() };
    let load_config = LoadCargoConfig {
        load_out_dirs_from_check: false,
        with_proc_macro_server: ProcMacroServerChoice::None,
        prefill_caches: false,
    };

    let (db, vfs, _proc_macro) = load_workspace_at(
        workspace_dir,
        &cargo_config,
        &load_config,
        &|_msg| {},
    )
    .context("Failed to load workspace")?;

    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

    let target_packages: HashSet<String> = if !packages.is_empty() {
        packages.iter().cloned().collect()
    } else {
        HashSet::new()
    };

    // Wrap all semantic analysis in attach_db — required for the trait solver's TLS.
    let units_by_file = attach_db(&db, || {
        let sema = Semantics::new(&db);

        let all_crates = Crate::all(&db);
        let target_crates: Vec<Crate> = all_crates
            .into_iter()
            .filter(|krate| {
                let name = krate
                    .display_name(&db)
                    .map(|n| n.to_string())
                    .unwrap_or_default();

                let normalized_name = name.replace('-', "_");
                if target_packages.is_empty() {
                    is_local_crate(&db, krate, &vfs, &cwd)
                } else {
                    target_packages.iter().any(|pkg| {
                        let normalized_pkg = pkg.replace('-', "_");
                        normalized_pkg == normalized_name
                    })
                }
            })
            .collect();

        visitor::extract_units(&db, &sema, &vfs, &target_crates, &cwd)
    });

    let timestamp = generation_id.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });

    for (source_path, mut units) in units_by_file {
        // rust-analyzer names each integration-test/example/bench crate after
        // its file stem, so ids like `extraction::extract` collide across
        // packages; qualify them by the owning package (ADR-0005).
        test_crate::qualify_test_units(&source_path, &mut units);

        let entry = FileCacheEntry {
            source_path: PathBuf::from(&source_path),
            units,
            cached_at: timestamp,
        };

        let output_file =
            cache_file_path(cache_dir, Path::new(&source_path), "json");

        if let Some(parent) = output_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match serde_json::to_string_pretty(&entry) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&output_file, json) {
                    eprintln!(
                        "Failed to write output for {}: {}",
                        source_path, e
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "Failed to serialize output for {}: {}",
                    source_path, e
                );
            }
        }

        // Write token cache for CPD
        let abs_source_path = workspace_dir.join(&source_path);
        if let Ok(source_text) = std::fs::read_to_string(&abs_source_path) {
            let file_tokens = tokenizer::tokenize_rust(
                &source_text,
                &source_path,
                timestamp,
            );
            let token_bytes = mdlr_cpd::binary::serialize(&file_tokens);
            let token_file =
                cache_file_path(cache_dir, Path::new(&source_path), "tokens");
            if let Err(e) = std::fs::write(&token_file, token_bytes) {
                eprintln!("Failed to write tokens for {}: {}", source_path, e);
            }
        }
    }

    Ok(())
}

/// Check if a crate has source files under the current working directory.
/// This is a heuristic for detecting workspace members vs external dependencies.
fn is_local_crate(
    db: &ra_ap_ide_db::RootDatabase,
    krate: &Crate,
    vfs: &ra_ap_vfs::Vfs,
    cwd: &Path,
) -> bool {
    let root_file = krate.root_file(db);
    let vfs_path = vfs.file_path(root_file);
    if let Some(abs_path) = vfs_path.as_path() {
        let file_path: &Path = abs_path.as_ref();
        file_path.starts_with(cwd)
    } else {
        false
    }
}
