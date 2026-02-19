use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "virgil", about = "Parse codebases and query structured parquet files")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Parse a codebase and output parquet files
    Parse {
        /// Root directory to parse
        dir: PathBuf,

        /// Output directory for parquet files
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Comma-separated language filter (ts,tsx,js,jsx)
        #[arg(short, long)]
        language: Option<String>,
    },

    /// Show codebase overview (semantic structure, module tree, API surface)
    Overview {
        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        /// Maximum directory depth for module tree
        #[arg(long, default_value = "3")]
        depth: usize,
    },

    /// Search for symbols by name
    Search {
        /// Search query (fuzzy match)
        query: String,

        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Filter by symbol kind
        #[arg(long)]
        kind: Option<String>,

        /// Only show exported symbols
        #[arg(long)]
        exported: bool,

        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,

        /// Number of results to skip
        #[arg(long, default_value = "0")]
        offset: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Show all symbols in a file
    Outline {
        /// File path to get outline for
        file_path: String,

        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// List parsed files
    Files {
        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Filter by language
        #[arg(long)]
        language: Option<String>,

        /// Filter by directory prefix
        #[arg(long)]
        directory: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "100")]
        limit: usize,

        /// Number of results to skip
        #[arg(long, default_value = "0")]
        offset: usize,

        /// Sort by field
        #[arg(long, default_value = "path")]
        sort: FileSortField,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Read source file content
    Read {
        /// File path to read (relative, as stored in parquet)
        file_path: String,

        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Root directory of the source project
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Start line (1-indexed)
        #[arg(long)]
        start_line: Option<usize>,

        /// End line (1-indexed, inclusive)
        #[arg(long)]
        end_line: Option<usize>,
    },

    /// Execute raw SQL against parquet files
    Query {
        /// SQL query to execute
        sql: String,

        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Show what a file imports (dependencies)
    Deps {
        /// File path to show dependencies for
        file_path: String,

        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Show what files import a given file (reverse dependencies)
    Dependents {
        /// File path to find dependents for
        file_path: String,

        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Find which files import a specific symbol
    Callers {
        /// Symbol name to search for (fuzzy match)
        symbol_name: String,

        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// List all imports with filters
    Imports {
        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Filter by module specifier (fuzzy match)
        #[arg(long)]
        module: Option<String>,

        /// Filter by import kind (static, dynamic, require, re_export)
        #[arg(long)]
        kind: Option<String>,

        /// Filter by source file prefix
        #[arg(long)]
        file: Option<String>,

        /// Only show type-only imports
        #[arg(long)]
        type_only: bool,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum FileSortField {
    Path,
    Lines,
    Size,
    Imports,
    Dependents,
}
