use cargo::core::compiler::{CompileMode, Executor, Unit};
use cargo::core::{PackageId, Target};
use cargo::CargoResult;
use cargo_util::ProcessBuilder;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::sync::Mutex;

use crate::HirExtractCallbacks;

/// Custom executor that intercepts rustc invocations for target packages
/// and runs them through `rustc_driver` with `HirExtractCallbacks`.
pub struct HirExtractExecutor {
    /// Mapping from source path → output path (for target packages)
    mapping: HashMap<String, String>,
    /// Set of package names we want to extract from
    target_packages: HashSet<String>,
    /// Mutex to serialize rustc_driver calls (global state safety)
    driver_lock: Mutex<()>,
}

impl HirExtractExecutor {
    pub fn new(mapping: HashMap<String, String>, target_packages: HashSet<String>) -> Self {
        Self {
            mapping,
            target_packages,
            driver_lock: Mutex::new(()),
        }
    }

    fn is_target_package(&self, id: PackageId) -> bool {
        self.target_packages.contains(&id.name().to_string())
    }
}

impl Executor for HirExtractExecutor {
    fn exec(
        &self,
        cmd: &ProcessBuilder,
        id: PackageId,
        _target: &Target,
        _mode: CompileMode,
        on_stdout_line: &mut dyn FnMut(&str) -> CargoResult<()>,
        on_stderr_line: &mut dyn FnMut(&str) -> CargoResult<()>,
    ) -> CargoResult<()> {
        if !self.is_target_package(id) {
            // Non-target package: run rustc normally
            return cmd.exec_with_streaming(on_stdout_line, on_stderr_line, false)
                .map(|_| ());
        }

        // Target package: extract args from ProcessBuilder, run rustc_driver
        let _lock = self.driver_lock.lock().unwrap_or_else(|e| e.into_inner());

        // Build the argument list: [program, ...args]
        // Strip --error-format and --json flags since we're running in-process
        // and want human-readable output (cargo sets these for JSON parsing but
        // our in-process driver writes directly to stderr, bypassing cargo's
        // on_stderr_line callback).
        let program: OsString = cmd.get_program().to_owned();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        let mut driver_args: Vec<String> = Vec::with_capacity(1 + args.len());
        driver_args.push(program.to_string_lossy().to_string());
        let mut skip_next = false;
        for arg in &args {
            if skip_next {
                skip_next = false;
                continue;
            }
            if arg.starts_with("--error-format") || arg.starts_with("--json=") {
                continue;
            }
            if arg == "--json" {
                skip_next = true;
                continue;
            }
            driver_args.push(arg.clone());
        }

        // Set up environment variables that cargo would set.
        // The ProcessBuilder has env vars we need to propagate.
        // SAFETY: We hold the driver_lock mutex, serializing all executor calls,
        // so no concurrent threads are reading these env vars.
        for (key, val) in cmd.get_envs() {
            if let Some(val) = val {
                unsafe { std::env::set_var(key, val) };
            }
        }

        let mut callbacks = HirExtractCallbacks {
            mapping: self.mapping.clone(),
        };

        let result = rustc_driver::catch_fatal_errors(|| {
            rustc_driver::run_compiler(&driver_args, &mut callbacks);
        });

        match result {
            Ok(()) => Ok(()),
            Err(_) => Err(anyhow::anyhow!(
                "rustc compilation failed for package {}",
                id.name()
            )
            .into()),
        }
    }

    fn force_rebuild(&self, unit: &Unit) -> bool {
        self.target_packages
            .contains(&unit.pkg.name().to_string())
    }
}
