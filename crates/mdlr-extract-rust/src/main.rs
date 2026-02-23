#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod branches;
mod calls;
mod executor;
mod field_access;
mod visitor;

use anyhow::{Context, Result};
use clap::Parser;
use rustc_driver::Callbacks;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

/// Cached extraction data for a single source file.
/// Matches the `FileCacheEntry` format from `crates/mdlr/src/cache/types.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileCacheEntry {
    source_path: PathBuf,
    units: Vec<mdlr_core::Unit>,
    cached_at: u64,
}

/// mdlr-extract-rust: HIR-based Rust unit extraction.
///
/// Uses cargo-as-library to orchestrate compilation and intercept rustc
/// invocations for target packages, extracting HIR-level unit information.
#[derive(Parser, Debug)]
#[command(name = "mdlr-extract-rust")]
struct Cli {
    /// Path to the workspace Cargo.toml
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    /// Output directory for per-file JSON results (mirrors source tree structure)
    #[arg(long)]
    output: Option<PathBuf>,

    /// Package names to extract from (if empty, extracts from all workspace members)
    #[arg(long)]
    package: Vec<String>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("mdlr-extract-rust: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    run_standalone_mode(&cli)
}

/// Uses cargo-as-library to orchestrate compilation.
fn run_standalone_mode(cli: &Cli) -> Result<()> {
    let manifest_path =
        cli.manifest_path.as_ref().context("--manifest-path is required")?;

    let output_dir = cli.output.as_ref().context("--output is required")?;

    // Canonicalize the manifest path
    let manifest_path = manifest_path.canonicalize().with_context(|| {
        format!("Failed to resolve manifest path: {}", manifest_path.display())
    })?;

    // Ensure cargo-as-library uses the same rustc that our linked rustc_driver
    // came from. Without this, cargo may discover a different rustc (e.g. stable)
    // and deps compiled with that rustc will be incompatible with our in-process
    // rustc_driver (nightly), causing E0514 metadata mismatch errors.
    let sysroot = env!("MDLR_RUSTC_SYSROOT");
    let rustc_path = PathBuf::from(sysroot).join("bin").join("rustc");
    // SAFETY: called before spawning any threads (cargo hasn't started yet)
    unsafe { std::env::set_var("RUSTC", &rustc_path) };

    // Set up cargo's GlobalContext
    let gctx = cargo::GlobalContext::default()
        .context("Failed to create cargo GlobalContext")?;

    // Default shell is Verbose (shows "Running rustc..." for every unit).
    // Set to Normal to show only "Compiling"/"Checking"/"Finished" + progress bars.
    gctx.shell().set_verbosity(cargo::core::shell::Verbosity::Normal);

    // Create workspace
    let ws = cargo::core::Workspace::new(&manifest_path, &gctx)
        .context("Failed to create cargo Workspace")?;

    // Determine which packages to compile and extract from.
    let target_packages: HashSet<String> = if !cli.package.is_empty() {
        cli.package.iter().cloned().collect()
    } else {
        // Extract from all workspace members
        ws.members().map(|p| p.name().to_string()).collect()
    };

    // Set up compile options for check mode
    let mut compile_opts = cargo::ops::CompileOptions::new(
        &gctx,
        cargo::core::compiler::CompileMode::Check { test: false },
    )
    .context("Failed to create CompileOptions")?;

    // Always set the package spec explicitly — Packages::Default fails on
    // virtual manifests (workspace root with no [package]).
    compile_opts.spec = cargo::ops::Packages::Packages(
        target_packages.iter().cloned().collect(),
    );

    // Create the executor
    let exec: Arc<dyn cargo::core::compiler::Executor> = Arc::new(
        executor::HirExtractExecutor::new(output_dir.clone(), target_packages),
    );

    // Run compilation with our custom executor.
    // Don't treat errors as fatal — some packages may fail to compile but
    // extraction still runs for whatever succeeded (via after_expansion).
    if let Err(e) = cargo::ops::compile_with_exec(&ws, &compile_opts, &exec) {
        eprintln!("warning: {e:#}");
    }

    Ok(())
}

struct HirExtractCallbacks {
    /// Output directory — per-file results are written as `<output_dir>/<source_path>.json`
    output_dir: PathBuf,
}

impl Callbacks for HirExtractCallbacks {
    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> rustc_driver::Compilation {
        // Don't call tcx.analysis() — it raises a fatal error if the target
        // crate has ANY compilation errors, killing the entire process.
        // Instead, let typeck results be computed on demand per-function
        // in calls::extract_calls(). Functions with errors get partial
        // extraction; functions without errors get full call resolution.
        let units_by_file = visitor::extract_units(tcx);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        for (source_path, units) in units_by_file {
            let entry = FileCacheEntry {
                source_path: PathBuf::from(&source_path),
                units,
                cached_at: timestamp,
            };

            // Write to <output_dir>/<source_path>.json (mirroring source tree)
            let mut output_file = self.output_dir.join(&source_path);
            output_file.set_extension("json");

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
        }

        rustc_driver::Compilation::Stop
    }
}
