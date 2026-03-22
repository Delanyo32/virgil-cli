use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "virgil",
    about = "Parse and query codebases on-demand",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage and query projects
    Projects {
        #[command(subcommand)]
        command: ProjectCommand,
    },

    /// Start a persistent HTTP server for queries and audits
    Serve {
        /// S3 URI — load codebase from S3 at startup
        #[arg(long)]
        s3: String,

        /// Host to bind (use 0.0.0.0 for all interfaces)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to bind (use 0 for OS-assigned)
        #[arg(long, default_value = "0")]
        port: u16,

        /// Comma-separated language filter
        #[arg(short, long)]
        lang: Option<String>,

        /// Glob patterns to exclude (repeatable)
        #[arg(short, long)]
        exclude: Vec<String>,
    },

    /// Static analysis and tech debt detection
    #[command(
        args_conflicts_with_subcommands = true,
        subcommand_precedence_over_arg = true
    )]
    Audit {
        /// Root directory to analyze (runs all audits)
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        #[command(subcommand)]
        command: Option<AuditCommand>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjectCommand {
    /// Register a project for querying
    Create {
        /// Project name
        name: String,

        /// Root directory of the project
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        /// Glob patterns to exclude (repeatable)
        #[arg(short, long)]
        exclude: Vec<String>,

        /// Comma-separated language filter (ts,tsx,js,jsx,c,h,cpp,cc,cxx,hpp,cs,rs,py,pyi,go,java,php)
        #[arg(short, long)]
        lang: Option<String>,
    },

    /// List registered projects
    List,

    /// Remove a registered project
    Delete {
        /// Project name to delete
        name: String,
    },

    /// Query a project using the JSON query language
    ///
    /// Pass a query via --q (inline), --file (path), or pipe JSON to stdin.
    ///
    /// QUERY FIELDS:
    ///   find         Symbol kind(s): function, method, class, type, enum, struct,
    ///                trait, variable, constant, property, namespace, module, any
    ///   name         Glob string, {"contains": "..."}, or {"regex": "..."}
    ///   files        Glob pattern(s) to scope files: "src/**/*.ts" or ["a/**", "b/**"]
    ///   files_exclude  Glob pattern(s) to exclude files
    ///   visibility   "exported", "public", "private", "protected", "internal"
    ///   inside       Only symbols nested inside a parent with this name
    ///   has          Filter by comment/decorator text; {"not": "docstring"} for inverse
    ///   lines        {"min": N, "max": N} — filter by line count
    ///   body         true — include full source body in results
    ///   preview      N — include first N lines of each symbol
    ///   calls        "down" (callees), "up" (callers), or "both"
    ///   depth        Call graph traversal depth (default 1, max 5)
    ///   read         File path to read (returns content instead of symbols)
    ///                Combine with `lines` for a specific range
    ///
    /// EXAMPLES:
    ///   # Find all exported functions
    ///   virgil projects query myapp --q '{"find": "function", "visibility": "exported"}'
    ///
    ///   # Search by name pattern with preview
    ///   virgil projects query myapp --q '{"name": "handle*", "preview": 5}' --pretty
    ///
    ///   # Methods inside a specific class
    ///   virgil projects query myapp --q '{"find": "method", "inside": "AuthService"}'
    ///
    ///   # Large functions (50+ lines) in a directory
    ///   virgil projects query myapp --q '{"files": "src/api/**", "find": "function", "lines": {"min": 50}}'
    ///
    ///   # Functions missing docstrings
    ///   virgil projects query myapp --q '{"find": "function", "has": {"not": "docstring"}}'
    ///
    ///   # Name regex — all getters
    ///   virgil projects query myapp --q '{"name": {"regex": "^get[A-Z]"}}'
    ///
    ///   # Call graph — what does authenticate() call?
    ///   virgil projects query myapp --q '{"name": "authenticate", "calls": "down", "depth": 2}'
    ///
    ///   # Summary of an entire project
    ///   virgil projects query myapp --q '{}' --out summary --pretty
    ///
    ///   # Read a file
    ///   virgil projects query myapp --q '{"read": "src/main.rs"}' --pretty
    ///
    ///   # Read specific lines from a file
    ///   virgil projects query myapp --q '{"read": "src/main.rs", "lines": {"min": 10, "max": 25}}'
    ///
    ///   # File:line locations only
    ///   virgil projects query myapp --q '{"find": "class"}' --out locations
    #[command(verbatim_doc_comment)]
    Query {
        /// Project name (not needed with --s3)
        #[arg(conflicts_with = "s3")]
        name: Option<String>,

        /// S3 URI — reads codebase directly from S3, bypasses project registry
        #[arg(long)]
        s3: Option<String>,

        /// Comma-separated language filter (used with --s3)
        #[arg(short, long)]
        lang: Option<String>,

        /// Glob patterns to exclude (used with --s3, repeatable)
        #[arg(short, long)]
        exclude: Vec<String>,

        /// Inline JSON query
        #[arg(short, long)]
        q: Option<String>,

        /// Path to a JSON query file
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Output format
        #[arg(short, long, default_value = "outline")]
        out: QueryOutputFormat,

        /// Pretty-print JSON output
        #[arg(long)]
        pretty: bool,

        /// Maximum number of results
        #[arg(short, long, default_value = "100")]
        max: usize,
    },
}

#[derive(Debug, Clone, ValueEnum)]
pub enum QueryOutputFormat {
    Outline,
    Snippet,
    Full,
    Tree,
    Locations,
    Summary,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

#[derive(Subcommand, Debug)]
pub enum AuditCommand {
    /// Code quality analysis (tech debt, complexity, security)
    #[command(
        args_conflicts_with_subcommands = true,
        subcommand_precedence_over_arg = true
    )]
    CodeQuality {
        /// Root directory to analyze (runs all code quality checks)
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

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
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

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

    /// Scalability analysis (N+1 queries, sync blocking in async, memory leaks)
    Scalability {
        /// Root directory to analyze
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,

        /// Comma-separated pipeline filter (n_plus_one_queries,sync_blocking_in_async,memory_leak_indicators)
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

    /// Architecture analysis (module size, circular deps, dependency depth, API surface)
    Architecture {
        /// Root directory to analyze
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

        /// Comma-separated language filter
        #[arg(short, long)]
        language: Option<String>,

        /// Comma-separated pipeline filter (module_size_distribution,circular_dependencies,dependency_graph_depth,api_surface_area)
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
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

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
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

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
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

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
