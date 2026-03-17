use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "virgil",
    about = "Parse codebases and query structured parquet files"
)]
pub struct Cli {
    /// Use S3 storage (reads credentials from S3_ACCESS_KEY_ID, S3_SECRET_ACCESS_KEY, S3_BUCKET_NAME, S3_ENDPOINT, S3_REGION env vars)
    #[arg(long, global = true)]
    pub env: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage and query parsed project data
    Projects {
        #[command(subcommand)]
        command: ProjectCommand,
    },

    /// Static analysis and tech debt detection
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjectCommand {
    /// Parse a codebase and output parquet files
    Parse {
        /// Root directory to parse
        dir: PathBuf,

        /// Output directory for parquet files
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Comma-separated language filter (ts,tsx,js,jsx,c,h,cpp,cc,cxx,hpp,cs,rs,py,pyi,go,java,php)
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

        /// Filter by import kind (static, dynamic, require, re_export, include, using)
        #[arg(long)]
        kind: Option<String>,

        /// Filter by source file prefix
        #[arg(long)]
        file: Option<String>,

        /// Only show type-only imports
        #[arg(long)]
        type_only: bool,

        /// Only show external (library) imports
        #[arg(long)]
        external: bool,

        /// Only show internal (user code) imports
        #[arg(long)]
        internal: bool,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
    /// List parse errors
    Errors {
        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Filter by error type (parser_creation, file_read, parse_failure)
        #[arg(long)]
        error_type: Option<String>,

        /// Filter by language
        #[arg(long)]
        language: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// List comments with filters
    Comments {
        /// Directory containing parquet files
        #[arg(long, default_value = ".")]
        data_dir: PathBuf,

        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Filter by comment kind (line, block, doc)
        #[arg(long)]
        kind: Option<String>,

        /// Only show comments associated with a symbol
        #[arg(long)]
        documented: bool,

        /// Filter by associated symbol name (fuzzy match)
        #[arg(long)]
        symbol: Option<String>,

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

#[derive(Subcommand, Debug)]
pub enum AuditCommand {
    /// Code quality analysis (tech debt, complexity, security)
    #[command(args_conflicts_with_subcommands = true, subcommand_precedence_over_arg = true)]
    CodeQuality {
        /// Root directory to analyze (runs all code quality checks)
        dir: Option<PathBuf>,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        #[command(subcommand)]
        command: Option<CodeQualityCommand>,
    },

    /// Security vulnerability detection (unsafe memory, injection, race conditions)
    Security {
        /// Root directory to analyze
        dir: PathBuf,

        /// Comma-separated language filter (currently: rs, go)
        #[arg(short, long)]
        language: Option<String>,

        /// Comma-separated pipeline filter
        #[arg(long)]
        pipeline: Option<String>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        /// Findings per page
        #[arg(long, default_value = "20", alias = "limit")]
        per_page: usize,

        /// Page number (1-indexed)
        #[arg(long, default_value = "1")]
        page: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum CodeQualityCommand {
    /// Detect tech debt patterns in source code
    TechDebt {
        /// Root directory to analyze
        dir: PathBuf,

        /// Comma-separated language filter (currently: rs, go, py)
        #[arg(short, long)]
        language: Option<String>,

        /// Comma-separated pipeline filter (e.g., panic_detection)
        #[arg(long)]
        pipeline: Option<String>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        /// Findings per page
        #[arg(long, default_value = "20", alias = "limit")]
        per_page: usize,

        /// Page number (1-indexed)
        #[arg(long, default_value = "1")]
        page: usize,
    },

    /// Measure code complexity metrics (cyclomatic, cognitive, function length, comment ratio)
    Complexity {
        /// Root directory to analyze
        dir: PathBuf,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,

        /// Comma-separated pipeline filter (cyclomatic_complexity,function_length,cognitive_complexity,comment_to_code_ratio)
        #[arg(long)]
        pipeline: Option<String>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        /// Findings per page
        #[arg(long, default_value = "20", alias = "limit")]
        per_page: usize,

        /// Page number (1-indexed)
        #[arg(long, default_value = "1")]
        page: usize,
    },

    /// Detect code style issues (dead code, duplication, coupling)
    CodeStyle {
        /// Root directory to analyze
        dir: PathBuf,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,

        /// Comma-separated pipeline filter (dead_code,duplicate_code,coupling)
        #[arg(long)]
        pipeline: Option<String>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        /// Findings per page
        #[arg(long, default_value = "20", alias = "limit")]
        per_page: usize,

        /// Page number (1-indexed)
        #[arg(long, default_value = "1")]
        page: usize,
    },
}
