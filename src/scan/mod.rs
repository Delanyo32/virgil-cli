pub mod prompts; // stub: only init_prompts for now — Task 5 fills this in
pub mod provider;
pub mod report;
pub mod tools;

use crate::cli::ProviderKind;
use anyhow::Result;
use std::path::PathBuf;

pub struct ScanConfig {
    pub path: PathBuf,
    pub prompts: Option<PathBuf>,
    pub workers: usize,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub json: bool,
    pub output: Option<PathBuf>,
    pub rebuild: bool,
    pub lang: Option<String>,
}

/// Builds (or warm-opens) the fact store for `cfg.path` and returns it
/// with the loaded workspace. Task 6 adds the agent phase after this.
pub async fn run(cfg: ScanConfig) -> Result<()> {
    let (_store, _ws) = build_store(&cfg)?;
    println!("store ready (agents arrive in a later task)");
    Ok(())
}

pub(crate) fn build_store(
    cfg: &ScanConfig,
) -> Result<(crate::db::DbStore, crate::storage::workspace::Workspace)> {
    use crate::language::{self, Language};
    let root = cfg.path.canonicalize()?;
    let languages = match &cfg.lang {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };
    let ws = crate::storage::workspace::Workspace::load(&root, &languages, None)?;

    let project_id = root.to_string_lossy().to_string();
    let cache_path = crate::db::cache_dir_for_db(&project_id)?;
    if cfg.rebuild && cache_path.exists() {
        std::fs::remove_file(&cache_path)?;
    }
    let store = crate::db::DbStore::open_persistent(&cache_path)?;
    if store.fresh() {
        crate::graph::builder::GraphBuilder::new(&ws, &languages).build(&store)?;
        crate::db::populate(&store)?;
    }
    Ok((store, ws))
}
