use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "virgil-cli", about = "Parse codebases and output structured parquet files")]
pub struct Args {
    /// Root directory to parse
    pub dir: PathBuf,

    /// Output directory for parquet files
    #[arg(short, long, default_value = ".")]
    pub output: PathBuf,

    /// Comma-separated language filter (ts,tsx,js,jsx)
    #[arg(short, long)]
    pub language: Option<String>,
}
