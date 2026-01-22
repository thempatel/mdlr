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
    /// Show files that need analysis
    Todo {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Also show files with untagged units
        #[arg(long)]
        all: bool,
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,
    },
    /// Run analysis on a directory
    Analyze {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Force re-analysis of all files
        #[arg(long)]
        force: bool,
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,
    },
    /// Export the graph from cached analysis
    Export {
        /// Path to export from (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output format
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
}

#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}
