use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "virgil-cli",
    about = "AI code review — parses your repo into DuckDB, then AI agents review it",
    version
)]
pub struct Cli {
    /// Increase log verbosity (-v info, -vv debug, -vvv trace). Overridden by VIRGIL_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress all logs except errors.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

// LogFormat is deliberately gone: the Json variant existed for serve-mode log
// shippers (observability/mod.rs's own comment said so). Compact-only now;
// observability::init lost its format parameter.

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Ollama,
    Openrouter,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Parse a repo and run an AI code review over it.
    Scan {
        /// Root directory of the project to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Markdown prompt file or directory (replaces the built-in reviews)
        #[arg(long)]
        prompts: Option<PathBuf>,

        /// Max review agents running at once
        #[arg(long, default_value_t = 4)]
        workers: usize,

        /// AI provider
        #[arg(long, value_enum, default_value_t = ProviderKind::Openrouter)]
        provider: ProviderKind,

        /// Model id (defaults to z-ai/glm-4.6 for openrouter and claude-opus-5
        /// for anthropic; required for openai and ollama)
        #[arg(long)]
        model: Option<String>,

        /// Emit findings as JSON instead of the terminal report
        #[arg(long)]
        json: bool,

        /// Also write a markdown report to this path
        #[arg(long)]
        output: Option<PathBuf>,

        /// Force a fresh rebuild of the cached fact store
        #[arg(long)]
        rebuild: bool,

        // ponytail: no --exclude. The old flag was decorative — registry.rs
        // stored the globs in projects.json and nothing ever read them back.
        // File discovery already honours .gitignore via ignore::WalkBuilder.
        // Add it by threading an OverrideBuilder through discover_files +
        // Workspace::load when someone actually needs a non-gitignored exclude.
        /// Comma-separated file extensions to parse. One of: ts, tsx, js, jsx, c, h,
        /// cpp, cc, cxx, hpp, hxx, hh, cs, rs, py, pyi, go, java, php
        #[arg(short, long)]
        lang: Option<String>,
    },

    /// Copy the built-in review prompts into a directory so you can edit or extend them.
    InitPrompts {
        /// Destination directory (created if missing)
        dir: PathBuf,
    },

    /// Delete all cached databases from the OS cache directory
    /// (~/.cache/virgil on Linux, ~/Library/Caches/virgil on macOS).
    Clean,
}
