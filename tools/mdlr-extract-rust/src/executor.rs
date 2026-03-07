use cargo::CargoResult;
use cargo::core::compiler::{CompileMode, Executor};
use cargo::core::{PackageId, Target};
use cargo_util::ProcessBuilder;
use std::collections::HashSet;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::HirExtractCallbacks;

/// Custom executor that intercepts rustc invocations for target packages
/// and runs them through `rustc_driver` with `HirExtractCallbacks`.
pub struct HirExtractExecutor {
    /// Output directory for per-file JSON results
    output_dir: PathBuf,
    /// Set of package names we want to extract from
    target_packages: HashSet<String>,
    /// Mutex to serialize rustc_driver calls (global state safety)
    driver_lock: Mutex<()>,
    /// If set, stamp all cache entries with this generation ID
    generation_id: Option<u64>,
}

impl HirExtractExecutor {
    pub fn new(
        output_dir: PathBuf,
        target_packages: HashSet<String>,
        generation_id: Option<u64>,
    ) -> Self {
        Self {
            output_dir,
            target_packages,
            driver_lock: Mutex::new(()),
            generation_id,
        }
    }

    fn is_target_package(&self, id: PackageId) -> bool {
        self.target_packages.contains(&id.name().to_string())
    }
}

/// Build the rustc driver argument list from a ProcessBuilder, stripping
/// --error-format and --json flags (cargo sets these for JSON parsing but
/// our in-process driver writes directly to stderr).
fn prepare_driver_args(cmd: &ProcessBuilder) -> Vec<String> {
    let program = cmd.get_program().to_string_lossy().to_string();
    let args: Vec<String> =
        cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();

    let mut driver_args: Vec<String> = Vec::with_capacity(1 + args.len());
    driver_args.push(program);
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
    driver_args
}

/// RAII guard that redirects stderr to /dev/null and restores it on drop.
struct StderrSuppress {
    saved_fd: i32,
}

impl StderrSuppress {
    /// If `MDLR_QUIET_DIAGNOSTICS` is set, suppress stderr; otherwise no-op.
    fn maybe_suppress() -> Option<Self> {
        if std::env::var_os("MDLR_QUIET_DIAGNOSTICS").is_none() {
            return None;
        }
        let saved_fd = unsafe { libc::dup(2) };
        if let Ok(devnull) = std::fs::File::open("/dev/null") {
            unsafe { libc::dup2(devnull.as_raw_fd(), 2) };
        }
        Some(Self { saved_fd })
    }
}

impl Drop for StderrSuppress {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.saved_fd, 2);
            libc::close(self.saved_fd);
        }
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
        if !self.is_target_package(id) || _target.is_custom_build() {
            return cmd
                .exec_with_streaming(on_stdout_line, on_stderr_line, false)
                .map(|_| ());
        }

        let _lock = self.driver_lock.lock().unwrap_or_else(|e| e.into_inner());
        let driver_args = prepare_driver_args(cmd);

        // SAFETY: We hold the driver_lock mutex, serializing all executor calls,
        // so no concurrent threads are reading these env vars.
        for (key, val) in cmd.get_envs() {
            if let Some(val) = val {
                unsafe { std::env::set_var(key, val) };
            }
        }

        let mut callbacks = HirExtractCallbacks {
            output_dir: self.output_dir.clone(),
            generation_id: self.generation_id,
        };

        let _suppress = StderrSuppress::maybe_suppress();
        let result = rustc_driver::catch_fatal_errors(|| {
            rustc_driver::run_compiler(&driver_args, &mut callbacks);
        });
        drop(_suppress);

        if result.is_err() {
            eprintln!(
                "warning: compilation errors in package {}, extraction may be incomplete",
                id.name()
            );
        }

        Ok(())
    }
}
