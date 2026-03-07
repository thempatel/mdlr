//! Handlers for the `mdlr metrics` command.

use anyhow::{Result, bail};

use crate::cli::MetricsCommand;
use crate::config;

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
    ]
}

pub fn handle_metrics(command: MetricsCommand) -> Result<()> {
    match command {
        MetricsCommand::Ls => {
            for (name, description) in get_metric_descriptions() {
                println!("{}", name);
                println!("  {}", description);
                println!();
            }
        }
        MetricsCommand::Get { name } => {
            let descriptions = get_metric_descriptions();
            let metric = descriptions.iter().find(|(n, _)| *n == name);

            match metric {
                Some((name, description)) => {
                    println!("{}", name);
                    println!("  {}", description);
                    println!();

                    let config = config::load()?;
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
                }
                None => {
                    bail!(
                        "Unknown metric '{}'. Run 'mdlr metrics ls' to see available metrics.",
                        name
                    );
                }
            }
        }
    }

    Ok(())
}
