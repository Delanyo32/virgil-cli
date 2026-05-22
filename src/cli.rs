use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "virgil-cli",
    about = "Parse and query codebases on-demand",
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

    /// Query a project using Cozoscript
    ///
    /// Pass the query via exactly one of:
    ///   --cozoscript '<inline>'   inline Cozoscript
    ///   --file <path>             load Cozoscript from a file
    ///   --template <name>         a built-in template (see src/queries/builtin/)
    ///
    /// Bind parameters with --param key=value (repeatable). Integers and
    /// booleans are auto-coerced; everything else binds as a string.
    ///
    /// Queries that return columns (file, line, severity, pattern, message)
    /// are auto-formatted as audit findings; any other shape prints as rows.
    ///
    /// EXAMPLES:
    ///   # Built-in template with a parameter
    ///   virgil-cli projects query myapp --template find_function_by_name --param name=login
    ///
    ///   # Inline Cozoscript
    ///   virgil-cli projects query myapp --cozoscript '?[name] := *symbol{name}'
    ///
    ///   # Cozoscript from a file, with --pretty JSON output
    ///   virgil-cli projects query myapp --file query.cozoql --pretty
    ///
    ///   # Query an S3 codebase directly (no registration)
    ///   virgil-cli projects query --s3 s3://bucket/prefix --template find_cycles --lang rs
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

        /// Inline Cozoscript query
        #[arg(long, conflicts_with = "template")]
        cozoscript: Option<String>,

        /// Path to a Cozoscript file (.cozoql or any text file)
        #[arg(short, long, conflicts_with_all = ["template", "cozoscript"])]
        file: Option<PathBuf>,

        /// Built-in template name (see `src/queries/builtin/`)
        #[arg(long, conflicts_with_all = ["cozoscript", "file"])]
        template: Option<String>,

        /// Parameter binding for Cozoscript / template (repeatable). Format: key=value
        #[arg(long = "param", value_parser = parse_key_value)]
        params: Vec<(String, String)>,

        /// Force a fresh rebuild of the cached fact store, even if the
        /// workspace appears unchanged. Useful when the schema-version
        /// check misses a semantic change.
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
