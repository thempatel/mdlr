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
use std::collections::{HashMap, HashSet};
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

    /// Path to the JSON mapping file (source path → output path)
    #[arg(long)]
    mapping: Option<PathBuf>,

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
    let manifest_path = cli
        .manifest_path
        .as_ref()
        .context("--manifest-path is required")?;

    let mapping_path = cli
        .mapping
        .as_ref()
        .context("--mapping is required")?;

    // Load the mapping file
    let mapping_content = std::fs::read_to_string(mapping_path)
        .with_context(|| format!("Failed to read mapping: {}", mapping_path.display()))?;
    let mapping: HashMap<String, String> =
        serde_json::from_str(&mapping_content).context("Failed to parse mapping JSON")?;

    // Canonicalize the manifest path
    let manifest_path = manifest_path
        .canonicalize()
        .with_context(|| format!("Failed to resolve manifest path: {}", manifest_path.display()))?;

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
    // If --package was specified, use those. Otherwise, discover which workspace
    // members contain files listed in the mapping.
    let target_packages: HashSet<String> = if !cli.package.is_empty() {
        cli.package.iter().cloned().collect()
    } else {
        // Derive packages from the mapping: check which workspace member
        // directories contain the source paths in the mapping.
        let ws_root = ws.root().to_path_buf();
        let mut packages = HashSet::new();
        for member in ws.members() {
            let pkg_dir = member.root();
            let relative_dir = pkg_dir
                .strip_prefix(&ws_root)
                .unwrap_or(pkg_dir)
                .to_string_lossy();
            for source_path in mapping.keys() {
                if source_path.starts_with(relative_dir.as_ref()) {
                    packages.insert(member.name().to_string());
                    break;
                }
            }
        }
        if packages.is_empty() {
            // Fallback: all workspace members
            ws.members().map(|p| p.name().to_string()).collect()
        } else {
            packages
        }
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
    let exec: Arc<dyn cargo::core::compiler::Executor> =
        Arc::new(executor::HirExtractExecutor::new(mapping, target_packages));

    // Run compilation with our custom executor
    cargo::ops::compile_with_exec(&ws, &compile_opts, &exec)
        .context("cargo compilation failed")?;

    Ok(())
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
