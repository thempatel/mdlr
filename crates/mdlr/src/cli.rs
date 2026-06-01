use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mdlr")]
#[command(about = "Modularity analyzer for code")]
#[command(version = env!("MDLR_VERSION"))]
pub struct Cli {
    /// Project root directory (skips automatic discovery)
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run analysis and display metrics
    Check {
        /// Path or symbol to constrain analysis to. Can be a file, directory, or fully qualified symbol ID (e.g., 'src/main.rs::handle_check').
        target: Option<String>,
        /// Max opportunities to show per metric (-1 for all)
        #[arg(short, default_value = "10", allow_hyphen_values = true)]
        k: i32,
        /// Pretty print as aligned table
        #[arg(long)]
        pretty: bool,
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,
        /// Show timing breakdown for each phase
        #[arg(long)]
        timing: bool,
        /// Analyze all files even when on a branch (default: diff mode on branches, all on main/master)
        #[arg(short = 'A', long)]
        all: bool,
        /// Scope analysis to a specific directory (combines with diff/all mode)
        #[arg(short = 'f', long)]
        filter: Option<String>,
        /// Suppress progress display
        #[arg(short = 'q', long)]
        quiet: bool,
        /// LCOV coverage file(s) to overlay onto changed files. Repeatable.
        #[arg(long = "cov", value_name = "PATH")]
        cov: Vec<PathBuf>,
    },
    /// List supported metrics with descriptions
    Metrics {
        #[command(subcommand)]
        command: MetricsCommand,
    },
    /// Output a markdown prompt for agent consumption
    Prompt,
    /// List symbols (units) in a file or directory
    Ls {
        /// Path to list symbols from (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Filter by unit kind (function, method, struct, module)
        #[arg(long)]
        kind: Option<String>,
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,
    },
    /// Get the content of a symbol
    Get {
        /// Symbol ID to retrieve
        symbol: String,
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,
    },
    /// Ignore specific metrics for specific symbols to reduce false positives
    Ignore {
        /// Metric name to ignore (e.g., "fan_in", "lcom")
        metric: Option<String>,
        /// Symbol ID to ignore the metric for
        symbol: Option<String>,
        /// Remove an existing ignore instead of adding one
        #[arg(long)]
        remove: bool,
        /// List all ignores
        #[arg(long)]
        list: bool,
    },
}

#[derive(Subcommand)]
pub enum MetricsCommand {
    /// List all available metrics
    Ls,
    /// Get details about a specific metric including thresholds
    Get {
        /// Name of the metric to get details for
        name: String,
    },
}

#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}
