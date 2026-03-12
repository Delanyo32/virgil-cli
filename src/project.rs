use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub repo_path: String,
    pub data_path: String,
    pub created_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProjectMetadata {
    pub projects: Vec<ProjectEntry>,
}

pub fn virgil_home() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".virgil"))
}

pub fn metadata_path() -> Result<PathBuf> {
    Ok(virgil_home()?.join("projects.json"))
}

pub fn project_data_dir(name: &str) -> Result<PathBuf> {
    Ok(virgil_home()?.join("projects").join(name))
}

pub fn load_metadata() -> Result<ProjectMetadata> {
    let path = metadata_path()?;
    if !path.exists() {
        return Ok(ProjectMetadata::default());
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let meta: ProjectMetadata = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(meta)
}

pub fn save_metadata(meta: &ProjectMetadata) -> Result<()> {
    let path = metadata_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn find_project<'a>(meta: &'a ProjectMetadata, name: &str) -> Option<&'a ProjectEntry> {
    meta.projects.iter().find(|p| p.name == name)
}

pub fn validate_project_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("project name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') || name.contains(std::path::MAIN_SEPARATOR) {
        anyhow::bail!("project name cannot contain path separators");
    }
    if name.starts_with('.') {
        anyhow::bail!("project name cannot start with '.'");
    }
    Ok(())
}

pub fn derive_project_name(dir: &Path) -> Result<String> {
    let name = dir
        .file_name()
        .context("could not derive project name from directory")?
        .to_string_lossy()
        .into_owned();
    Ok(name)
}
