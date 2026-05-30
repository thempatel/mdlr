//! Handlers for the `mdlr metrics` command.

use anyhow::{Result, bail};

use crate::cache::CacheStore;
use crate::cli::MetricsCommand;
use crate::config;
use crate::find_project_root;
use std::path::Path;

fn get_metric_descriptions() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "dag_density",
            "Ratio of edges to nodes in the dependency graph. High values indicate tightly coupled code; low values suggest isolated components.",
        ),
        (
            "fan_in",
            "Number of incoming dependencies to a unit. High values indicate core/shared code; very high may signal a bottleneck.",
        ),
        (
            "fan_out",
            "Number of outgoing dependencies from a unit. High values indicate a unit with many responsibilities that may need refactoring.",
        ),
        (
            "function_size",
            "Function size in lines of code. High values suggest functions that are hard to understand and test.",
        ),
        (
            "params",
            "Number of parameters on a function. High values (>4) often indicate a function doing too much or needing a parameter object.",
        ),
        (
            "cyclomatic",
            "Cyclomatic complexity (branches + 1) of a function. High values indicate complex control flow that is harder to test and maintain.",
        ),
        (
            "cognitive",
            "Cognitive complexity of a function. Unlike cyclomatic complexity, penalizes nesting depth — a branch inside a loop inside a branch costs more than three flat branches. High values indicate code that is hard to understand.",
        ),
        (
            "max_scope",
            "Largest single scope block (if/else body, match arm, loop body) within a function in lines. High values indicate oversized blocks that should be extracted.",
        ),
        (
            "methods_per_struct",
            "Number of methods in a struct. High values may indicate a type with too many responsibilities.",
        ),
        (
            "lcom",
            "Lack of Cohesion of Methods (LCOM4). Counts connected components of methods sharing fields or calls. 1 = cohesive, 2+ = struct has unrelated groups and could be split.",
        ),
        (
            "file_loc",
            "Lines of code per file. High values indicate large files that may be hard to navigate and maintain.",
        ),
        (
            "duplication_pct",
            "Percentage of lines in a file that are part of a duplicated code block (copy-paste detection). High values indicate copy-pasted code that should be refactored into shared abstractions.",
        ),
        (
            "line_cov",
            "Per-function line coverage percentage (0-100), computed from an LCOV file passed via `--cov`. Each function's value is the share of its own DA-instrumented lines that ran at least once; lines inside nested units (closures, methods) attribute to the nested unit, not the parent. LOWER values are worse — a function reporting 0 may have no records in the lcov (stale or incomplete coverage run) or may genuinely have no tests.",
        ),
        (
            "uncov_branches",
            "Per-function count of LCOV BRDA records inside the function's span where `taken == 0` — branches that never fired in the test run. Only emitted when the input lcov contains BRDA records; omitted (with a hazard warning) otherwise. Higher values mean more untested code paths.",
        ),
    ]
}

pub fn handle_metrics(
    command: MetricsCommand,
    explicit_root: Option<&Path>,
) -> Result<()> {
    // Load config (if any) so we can flag disabled metrics. Falls back to
    // defaults when run outside a project.
    let root = find_project_root(Path::new("."), explicit_root);
    let config = CacheStore::open(&root)
        .and_then(|s| config::load_from_dir(s.root()))
        .unwrap_or_default();

    match command {
        MetricsCommand::Ls => {
            print_metric_list(&config);
            Ok(())
        }
        MetricsCommand::Get { name } => print_metric_detail(&name, &config),
    }
}

/// Print every metric with its description, flagging disabled ones.
fn print_metric_list(config: &config::Config) {
    for (name, description) in get_metric_descriptions() {
        let suffix =
            if config.is_disabled(name) { "  (disabled)" } else { "" };
        println!("{}{}", name, suffix);
        println!("  {}", description);
        println!();
    }
}

/// Print one metric's description, disabled state, and thresholds.
fn print_metric_detail(name: &str, config: &config::Config) -> Result<()> {
    let descriptions = get_metric_descriptions();
    let Some((name, description)) =
        descriptions.iter().find(|(n, _)| *n == name)
    else {
        bail!(
            "Unknown metric '{}'. Run 'mdlr metrics ls' to see available metrics.",
            name
        );
    };

    println!("{}", name);
    println!("  {}", description);
    if config.is_disabled(name) {
        println!("  (disabled — suppressed from check output)");
    }
    println!();

    if let Some(t) = config.thresholds.get(name) {
        println!("thresholds:");
        println!("  excellent  < {}", t.excellent);
        println!("  good       < {}", t.good);
        println!("  fair       < {}", t.fair);
        println!("  poor       < {}", t.poor);
        println!("  critical   >= {}", t.poor);
    } else {
        println!("(no thresholds defined)");
    }
    Ok(())
}
