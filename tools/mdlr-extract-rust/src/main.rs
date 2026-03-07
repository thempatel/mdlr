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
mod scopes;
mod visitor;
mod walk;

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

    /// Generation ID to stamp on all cache entries (used for stale-entry filtering)
    #[arg(long)]
    generation_id: Option<u64>,
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

/// Creates a cargo GlobalContext configured for mdlr extraction.
fn create_cargo_context() -> Result<cargo::GlobalContext> {
    let gctx = cargo::GlobalContext::default()
        .context("Failed to create cargo GlobalContext")?;

    // Default shell is Verbose (shows "Running rustc..." for every unit).
    // Set to Normal to show only "Compiling"/"Checking"/"Finished" + progress bars.
    gctx.shell().set_verbosity(cargo::core::shell::Verbosity::Normal);

    Ok(gctx)
}

/// Opens a cargo Workspace with a separate .mdlr/target dir.
fn open_workspace<'gctx>(
    manifest_path: &std::path::Path,
    gctx: &'gctx cargo::GlobalContext,
) -> Result<cargo::core::Workspace<'gctx>> {
    let mut ws = cargo::core::Workspace::new(manifest_path, gctx)
        .context("Failed to create cargo Workspace")?;

    // Use a separate target directory (.mdlr/target) so that mdlr's check-mode
    // builds don't invalidate the user's normal build cache in target/.
    let mdlr_target_dir = ws.root().join(".mdlr").join("target");
    ws.set_target_dir(cargo::util::Filesystem::new(mdlr_target_dir));

    Ok(ws)
}

/// Resolves which packages to extract: CLI-specified or all workspace members.
fn resolve_target_packages(
    cli: &Cli,
    ws: &cargo::core::Workspace<'_>,
) -> HashSet<String> {
    if !cli.package.is_empty() {
        cli.package.iter().cloned().collect()
    } else {
        ws.members().map(|p| p.name().to_string()).collect()
    }
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

    let gctx = create_cargo_context()?;
    let ws = open_workspace(&manifest_path, &gctx)?;
    let target_packages = resolve_target_packages(cli, &ws);

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
    let generation_id = cli.generation_id;
    let exec: Arc<dyn cargo::core::compiler::Executor> =
        Arc::new(executor::HirExtractExecutor::new(
            output_dir.clone(),
            target_packages,
            generation_id,
        ));

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
    /// If set, use this value as `cached_at` instead of the current time.
    generation_id: Option<u64>,
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

        let timestamp = self.generation_id.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });

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

        rustc_driver::Compilation::Continue
    }
}
