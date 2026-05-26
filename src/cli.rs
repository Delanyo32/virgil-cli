use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "virgil-cli",
    about = "Parse and query codebases on-demand (DuckDB backend — experimental)",
    version
)]
pub struct Cli {
    /// Increase log verbosity (-v info, -vv debug, -vvv trace). Overridden by VIRGIL_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress all logs except errors.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Log output format.
    #[arg(long, global = true, value_enum, default_value_t = LogFormat::Compact)]
    pub log_format: LogFormat,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LogFormat {
    Compact,
    Json,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage and query projects
    Projects {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    // `Serve` subcommand and `--s3` flag are dropped on this branch —
    // see docs/experiments/duckdb-swap.md (Q9 decision: local CLI only).
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

    /// Query a project using SQL (with PGQ extensions for graph templates)
    ///
    /// Pass the query via exactly one of:
    ///   --sql '<inline>'       inline SQL
    ///   --file <path>          load SQL from a file
    ///   --template <name>      a built-in template (see src/queries/builtin/)
    ///
    /// Bind parameters with --param key=value (repeatable). Integers and
    /// booleans are auto-coerced; everything else binds as a string.
    /// Templates reference parameters as $name.
    ///
    /// Queries that return columns (file, line, severity, pattern, message)
    /// are auto-formatted as audit findings; any other shape prints as rows.
    ///
    /// EXAMPLES:
    ///   # Built-in template with a parameter
    ///   virgil-cli projects query myapp --template find_function_by_name --param name=login
    ///
    ///   # Inline SQL
    ///   virgil-cli projects query myapp --sql 'SELECT name FROM symbol LIMIT 10'
    #[command(verbatim_doc_comment)]
    Query {
        /// Project name
        name: String,

        /// Comma-separated language filter
        #[arg(short, long)]
        lang: Option<String>,

        /// Glob patterns to exclude (repeatable)
        #[arg(short, long)]
        exclude: Vec<String>,

        /// Inline SQL query
        #[arg(long, conflicts_with = "template")]
        sql: Option<String>,

        /// Path to a SQL file (.sql or any text file)
        #[arg(short, long, conflicts_with_all = ["template", "sql"])]
        file: Option<PathBuf>,

        /// Built-in template name (see `src/queries/builtin/`)
        #[arg(long, conflicts_with_all = ["sql", "file"])]
        template: Option<String>,

        /// Parameter binding for SQL / template (repeatable). Format: key=value
        #[arg(long = "param", value_parser = parse_key_value)]
        params: Vec<(String, String)>,

        /// Force a fresh rebuild of the cached fact store.
        #[arg(long)]
        rebuild: bool,

        /// Pretty-print JSON output
        #[arg(long)]
        pretty: bool,
    },
}

fn parse_key_value(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got '{s}'"))
}
