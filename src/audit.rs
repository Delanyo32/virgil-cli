use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::project::virgil_home;

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub name: String,
    pub repo_path: String,
    pub data_path: String,
    pub created_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AuditMetadata {
    pub audits: Vec<AuditEntry>,
}

pub fn audit_metadata_path() -> Result<PathBuf> {
    Ok(virgil_home()?.join("audits.json"))
}

pub fn audit_data_dir(name: &str) -> Result<PathBuf> {
    Ok(virgil_home()?.join("audits").join(name))
}

pub fn load_audit_metadata() -> Result<AuditMetadata> {
    let path = audit_metadata_path()?;
    if !path.exists() {
        return Ok(AuditMetadata::default());
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let meta: AuditMetadata = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(meta)
}

pub fn save_audit_metadata(meta: &AuditMetadata) -> Result<()> {
    let path = audit_metadata_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn find_audit<'a>(meta: &'a AuditMetadata, name: &str) -> Option<&'a AuditEntry> {
    meta.audits.iter().find(|a| a.name == name)
}

pub fn validate_audit_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("audit name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') || name.contains(std::path::MAIN_SEPARATOR) {
        anyhow::bail!("audit name cannot contain path separators");
    }
    if name.starts_with('.') {
        anyhow::bail!("audit name cannot start with '.'");
    }
    Ok(())
}

pub fn derive_audit_name(dir: &Path) -> Result<String> {
    let name = dir
        .file_name()
        .context("could not derive audit name from directory")?
        .to_string_lossy()
        .into_owned();
    Ok(name)
}
