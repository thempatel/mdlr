use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};

mod cache;
mod check;
mod check_output;
mod cli;
mod config;
mod extraction;
mod git_diff;
mod ignore_commands;
mod json_output;
mod metrics_commands;
mod metrics_rows;
mod progress;
mod symbol_commands;
mod timing;
mod walk;

use cli::{Cli, Command};
use symbol_commands::{handle_get, handle_ls};

/// Resolve the project root: use the explicit root if provided, otherwise walk up
/// from `start_dir` and find the highest directory with both `.mdlr` and `.git`.
/// Falls back to `start_dir` if none found.
pub fn find_project_root(
    start_dir: &Path,
    explicit_root: Option<&Path>,
) -> PathBuf {
    if let Some(root) = explicit_root {
        return root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    }

    let start =
        start_dir.canonicalize().unwrap_or_else(|_| start_dir.to_path_buf());
    let mut current = start.as_path();
    let mut highest: Option<&Path> = None;

    loop {
        if current.join(".mdlr").exists() && current.join(".git").exists() {
            highest = Some(current);
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }

    highest.map(|p| p.to_path_buf()).unwrap_or(start)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = cli.root;

    match cli.command {
        Command::Check {
            target,
            k,
            pretty,
            format,
            timing,
            all,
            filter,
            quiet,
            cov,
        } => check::handle_check(check::CheckArgs {
            target,
            k,
            pretty,
            format,
            timing,
            all,
            filter,
            quiet,
            cov,
            root,
        }),
        Command::Metrics { command } => {
            metrics_commands::handle_metrics(command, root.as_deref())
        }
        Command::Prompt => handle_prompt(),
        Command::Ls { path, kind, format } => {
            handle_ls(&path, kind, format, root.as_deref())
        }
        Command::Get { symbol, format } => {
            handle_get(&symbol, format, root.as_deref())
        }
        Command::Ignore { metric, symbol, remove, list } => {
            ignore_commands::handle_ignore(
                metric,
                symbol,
                remove,
                list,
                root.as_deref(),
            )
        }
    }
}

fn handle_prompt() -> Result<()> {
    print!("{}", include_str!("prompt.md"));
    Ok(())
}
