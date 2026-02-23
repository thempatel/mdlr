use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mdlr")]
#[command(about = "Modularity analyzer for code")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run analysis and display metrics
    Check {
        /// Path or symbol to constrain analysis to. Can be a file, directory, or fully qualified symbol ID (e.g., 'src/main.rs::handle_check').
        target: Option<String>,
        /// Save extraction results to cache (by default, check is read-only)
        #[arg(long)]
        save: bool,
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
    /// Manage semantic tags on symbols
    Tag {
        /// Symbol ID to tag (required unless --list is used)
        symbol: Option<String>,
        /// Add tags to the symbol (can be used multiple times)
        #[arg(long)]
        add: Vec<String>,
        /// Remove a tag from the symbol
        #[arg(long)]
        remove: Option<String>,
        /// Clear all tags from the symbol
        #[arg(long)]
        clear: bool,
        /// List all semantic tags in the project
        #[arg(long)]
        list: bool,
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
