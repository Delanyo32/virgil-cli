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
        #[arg(long, required_unless_present = "dir", conflicts_with = "dir")]
        s3: Option<String>,

        /// Local directory — load codebase from disk at startup (alternative to --s3)
        #[arg(long, required_unless_present = "s3", conflicts_with = "s3")]
        dir: Option<PathBuf>,

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
    Audit {
        /// Root directory to analyze
        #[arg(conflicts_with = "s3")]
        dir: Option<PathBuf>,

        /// S3 URI — reads codebase directly from S3
        #[arg(long, conflicts_with = "dir")]
        s3: Option<String>,

        /// Comma-separated language filter (rs,go,py,ts,js,java,php,cs,c,cpp)
        #[arg(short, long)]
        language: Option<String>,

        /// Filter by category: security, architecture, code_style, tech_debt, complexity, scalability
        #[arg(long)]
        category: Option<String>,

        /// Comma-separated pipeline name filter
        #[arg(long)]
        pipeline: Option<String>,

        /// Output format
        #[arg(long, default_value = "table")]
        format: OutputFormat,

        /// Run a specific JSON audit file instead of (or in addition to) built-ins
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,

        /// Findings per page
        #[arg(long, default_value = "20")]
        per_page: usize,

        /// Page number (1-indexed)
        #[arg(long, default_value = "1")]
        page: usize,
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
