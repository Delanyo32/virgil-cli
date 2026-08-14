use anyhow::Result;
use clap::Parser;
use tracing::warn;

use virgil_cli::cli::{Cli, Command};
use virgil_cli::observability;

fn main() -> Result<()> {
    let cli = Cli::parse();
    observability::init(cli.verbose, cli.quiet);

    let result = dispatch(cli.command);
    if let Err(err) = &result {
        warn!(error = %err, "command failed");
    }
    result
}

fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Scan {
            path,
            prompts,
            workers,
            provider,
            model,
            json,
            output,
            rebuild,
            lang,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(virgil_cli::scan::run(virgil_cli::scan::ScanConfig {
                path,
                prompts,
                workers,
                provider,
                model,
                json,
                output,
                rebuild,
                lang,
            }))
        }
        Command::InitPrompts { dir } => virgil_cli::scan::prompts::init_prompts(&dir),
        Command::Clean => {
            // Same helper `scan` writes through, so the two cannot drift.
            let dir = virgil_cli::db::cache_root()?;
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
                println!("removed {}", dir.display());
            } else {
                println!("nothing to clean");
            }
            Ok(())
        }
    }
}
