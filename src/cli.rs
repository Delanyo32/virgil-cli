use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "virgil", about = "Parse and query codebases on-demand")]
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

    /// Static analysis and tech debt detection
    #[command(
        args_conflicts_with_subcommands = true,
        subcommand_precedence_over_arg = true
    )]
    Audit {
        /// Root directory to analyze (runs all audits)
        dir: Option<PathBuf>,

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
    Query {
        /// Project name
        name: String,

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

    /// Scalability analysis (N+1 queries, sync blocking in async, memory leaks)
    Scalability {
        /// Root directory to analyze
        dir: PathBuf,

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
        dir: PathBuf,

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
