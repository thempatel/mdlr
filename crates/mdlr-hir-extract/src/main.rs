#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod branches;
mod calls;
mod field_access;
mod visitor;

use anyhow::{Context, Result, bail};
use rustc_driver::Callbacks;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Cached extraction data for a single source file.
/// Matches the `FileCacheEntry` format from `crates/mdlr/src/cache/types.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileCacheEntry {
    source_path: PathBuf,
    units: Vec<mdlr_core::Unit>,
    cached_at: u64,
}

/// This binary is used as a `RUSTC_WRAPPER`. Cargo invokes it in place of rustc
/// for every compilation unit.
///
/// Environment variables (set by the orchestrating CLI):
///   MDLR_HIR_MAPPING  — path to JSON file mapping source paths → output paths
///   MDLR_HIR_CRATE    — cargo package name of the crate to extract from
///
/// When cargo calls us:
///   args[0] = this binary, args[1] = real rustc, args[2..] = rustc flags
///
/// For non-target crates we exec the real rustc directly.
/// For the target crate we run rustc through our callbacks to extract HIR.
fn main() {
    if let Err(e) = run() {
        eprintln!("mdlr-hir-extract: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        bail!("Expected to be invoked as RUSTC_WRAPPER (first arg should be the real rustc path)");
    }

    let real_rustc = &args[1];
    let rustc_args = &args[2..];

    // Which crate is cargo compiling right now?
    let compiling_crate = rustc_args
        .iter()
        .position(|a| a == "--crate-name")
        .and_then(|i| rustc_args.get(i + 1))
        .map(|s| s.as_str());

    // Cargo passes crate names with hyphens replaced by underscores to rustc.
    let target_crate = std::env::var("MDLR_HIR_CRATE").ok();
    let target_normalized = target_crate.as_deref().map(|s| s.replace('-', "_"));

    let is_target = matches!(
        (&compiling_crate, &target_normalized),
        (Some(compiling), Some(target)) if *compiling == target
    );

    if !is_target {
        // Pass through to real rustc.
        let status = std::process::Command::new(real_rustc)
            .args(rustc_args)
            .status()
            .context("Failed to run real rustc")?;
        std::process::exit(status.code().unwrap_or(1));
    }

    // Target crate — load the mapping and run the compiler with our callbacks.
    let mapping_path =
        std::env::var("MDLR_HIR_MAPPING").context("MDLR_HIR_MAPPING env var not set")?;
    let mapping_content = std::fs::read_to_string(&mapping_path)
        .with_context(|| format!("Failed to read mapping: {}", mapping_path))?;
    let mapping: HashMap<String, String> =
        serde_json::from_str(&mapping_content).context("Failed to parse mapping JSON")?;

    let mut callbacks = HirExtractCallbacks { mapping };

    // args for run_compiler: [rustc, ...flags]  (first element is treated as argv[0])
    let mut driver_args = vec![real_rustc.clone()];
    driver_args.extend_from_slice(rustc_args);

    let result = rustc_driver::catch_fatal_errors(|| {
        rustc_driver::run_compiler(&driver_args, &mut callbacks);
    });

    match result {
        Ok(()) => Ok(()),
        Err(_) => bail!("rustc compilation failed"),
    }
}

struct HirExtractCallbacks {
    mapping: HashMap<String, String>,
}

impl Callbacks for HirExtractCallbacks {
    fn after_analysis(&mut self, _compiler: &Compiler, tcx: TyCtxt<'_>) -> rustc_driver::Compilation {
        let units_by_file = visitor::extract_units(tcx, &self.mapping);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        for (source_path, units) in units_by_file {
            let output_path = match self.mapping.get(&source_path) {
                Some(p) => PathBuf::from(p),
                None => continue,
            };

            let entry = FileCacheEntry {
                source_path: PathBuf::from(&source_path),
                units,
                cached_at: timestamp,
            };

            if let Some(parent) = output_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            match serde_json::to_string_pretty(&entry) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&output_path, json) {
                        eprintln!("Failed to write output for {}: {}", source_path, e);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to serialize output for {}: {}", source_path, e);
                }
            }
        }

        rustc_driver::Compilation::Stop
    }
}
