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
    /// Manage persistent projects (create, list, delete, query)
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },

    /// Run code audits (complexity analysis)
    Audit {
        #[command(subcommand)]
        action: AuditAction,
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
pub enum ProjectAction {
    /// Parse a codebase and register it as a named project
    Create {
        /// Directory to parse
        dir: PathBuf,

        /// Custom project name (defaults to directory basename)
        #[arg(short, long)]
        name: Option<String>,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,
    },

    /// List all registered projects
    List,

    /// Delete a project and its data
    Delete {
        /// Project name
        name: String,
    },

    /// Query a project by name
    Query {
        /// Project name
        name: String,

        #[command(subcommand)]
        command: ProjectQueryCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjectQueryCommand {
    /// Show codebase overview
    Overview {
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

    /// File operations
    File {
        #[command(subcommand)]
        action: FileAction,
    },

    /// Symbol operations
    Symbol {
        #[command(subcommand)]
        action: SymbolAction,
    },

    /// Comment operations
    Comments {
        #[command(subcommand)]
        action: CommentsAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum FileAction {
    /// Show all symbols in a file (outline)
    Get {
        /// File path
        path: String,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// List parsed files
    List {
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
        /// File path
        path: String,

        /// Start line (1-indexed)
        #[arg(long)]
        start_line: Option<usize>,

        /// End line (1-indexed, inclusive)
        #[arg(long)]
        end_line: Option<usize>,
    },
}

#[derive(Subcommand, Debug)]
pub enum SymbolAction {
    /// Get symbol details (type, usage, deps, callers)
    Get {
        /// Symbol name
        name: String,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
}

#[derive(Subcommand, Debug)]
pub enum CommentsAction {
    /// List comments with filters
    List {
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

    /// Search comments by text content
    Search {
        /// Search query (text match)
        query: String,

        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Filter by comment kind (line, block, doc)
        #[arg(long)]
        kind: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
}

#[derive(Subcommand, Debug)]
pub enum AuditAction {
    /// Parse a codebase and create a named audit
    Create {
        /// Directory to parse
        dir: PathBuf,

        /// Custom audit name (defaults to directory basename)
        #[arg(short, long)]
        name: Option<String>,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,
    },

    /// List all registered audits
    List,

    /// Delete an audit and its data
    Delete {
        /// Audit name
        name: String,
    },

    /// View complexity metrics for symbols
    Complexity {
        /// Audit name
        name: String,

        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Filter by symbol kind (function, method, arrow_function)
        #[arg(long)]
        kind: Option<String>,

        /// Sort by field
        #[arg(long, default_value = "cyclomatic")]
        sort: ComplexitySortField,

        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,

        /// Only show symbols with cyclomatic complexity >= threshold
        #[arg(long)]
        threshold: Option<u32>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Show audit overview (complexity + quality summary)
    Overview {
        /// Audit name
        name: String,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Analyze code quality (dead code, coupling, duplication)
    Quality {
        /// Audit name
        name: String,

        #[command(subcommand)]
        command: QualityCommand,
    },

    /// Analyze security issues (unsafe calls, string risks, hardcoded secrets)
    Security {
        /// Audit name
        name: String,

        #[command(subcommand)]
        command: SecurityCommand,
    },

    /// Detect common programming antipatterns
    Antipatterns {
        /// Audit name
        name: String,

        #[command(subcommand)]
        command: AntipatternCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum QualityCommand {
    /// Find exported symbols with no internal imports
    DeadCode {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Filter by symbol kind
        #[arg(long)]
        kind: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Analyze file coupling and dependency cycles
    Coupling {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Sort by field
        #[arg(long, default_value = "instability")]
        sort: CouplingSortField,

        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,

        /// Show circular dependencies
        #[arg(long)]
        cycles: bool,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Find structurally similar functions (DRY violations)
    Duplication {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Minimum group size (default 2 = pairs)
        #[arg(long, default_value = "2")]
        min_group: usize,

        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
}

#[derive(Subcommand, Debug)]
pub enum SecurityCommand {
    /// Find calls to dangerous functions (eval, exec, system, etc.)
    UnsafeCalls {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Find string literals containing SQL or HTML patterns
    StringRisks {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Find variables with secret-like names set to string literals
    HardcodedSecrets {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
}

#[derive(Subcommand, Debug)]
pub enum AntipatternCommand {
    /// Show all detected antipatterns
    All {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Filter by category (type_safety, error_handling, correctness, maintainability)
        #[arg(long)]
        category: Option<String>,

        /// Filter by severity (high, medium, low)
        #[arg(long)]
        severity: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Detect type safety issues (any types, type assertions, non-null assertions)
    TypeSafety {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Detect error handling issues (unwrap, panic, bare except, empty catch)
    ErrorHandling {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Detect correctness issues (var declarations, loose equality, mutable defaults)
    Correctness {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },

    /// Detect maintainability issues (global statements, wildcard imports, namespace pollution)
    Maintainability {
        /// Filter by file path prefix
        #[arg(long)]
        file: Option<String>,

        /// Maximum results to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, ValueEnum)]
pub enum CouplingSortField {
    Instability,
    FanIn,
    FanOut,
    File,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum ComplexitySortField {
    Cyclomatic,
    Cognitive,
    Name,
    File,
    Lines,
}
