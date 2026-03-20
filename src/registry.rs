use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::discovery;
use crate::language::{self, Language};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub path: PathBuf,
    pub exclude: Vec<String>,
    pub languages: Option<String>,
    pub file_count: usize,
    pub language_breakdown: HashMap<String, usize>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProjectRegistry {
    pub projects: Vec<ProjectEntry>,
}

pub fn registry_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let dir = home.join(".virgil-cli");
    Ok(dir.join("projects.json"))
}

pub fn load_registry() -> Result<ProjectRegistry> {
    let path = registry_path()?;
    if !path.exists() {
        return Ok(ProjectRegistry::default());
    }
    let data =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let reg: ProjectRegistry =
        serde_json::from_str(&data).with_context(|| "failed to parse projects.json")?;
    Ok(reg)
}

pub fn save_registry(reg: &ProjectRegistry) -> Result<()> {
    let path = registry_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(reg)?;
    fs::write(&tmp, &data).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| "failed to rename temp registry file")?;
    Ok(())
}

pub fn create_project(
    name: &str,
    path: PathBuf,
    exclude: Vec<String>,
    lang_filter: Option<&str>,
) -> Result<ProjectEntry> {
    let mut reg = load_registry()?;

    if reg.projects.iter().any(|p| p.name == name) {
        bail!("project '{}' already exists", name);
    }

    let canonical = fs::canonicalize(&path)
        .with_context(|| format!("path does not exist: {}", path.display()))?;

    let languages = match lang_filter {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };

    let files = discovery::discover_files(&canonical, &languages)?;

    let mut breakdown: HashMap<String, usize> = HashMap::new();
    for file in &files {
        if let Some(ext) = file.extension().and_then(|e| e.to_str())
            && let Some(lang) = Language::from_extension(ext) {
                *breakdown.entry(lang.as_str().to_string()).or_default() += 1;
            }
    }

    let entry = ProjectEntry {
        name: name.to_string(),
        path: canonical,
        exclude,
        languages: lang_filter.map(|s| s.to_string()),
        file_count: files.len(),
        language_breakdown: breakdown,
        created_at: Utc::now(),
    };

    reg.projects.push(entry.clone());
    save_registry(&reg)?;
    Ok(entry)
}

pub fn list_projects() -> Result<Vec<ProjectEntry>> {
    let reg = load_registry()?;
    Ok(reg.projects)
}

pub fn delete_project(name: &str) -> Result<()> {
    let mut reg = load_registry()?;
    let len_before = reg.projects.len();
    reg.projects.retain(|p| p.name != name);
    if reg.projects.len() == len_before {
        bail!("project '{}' not found", name);
    }
    save_registry(&reg)?;
    Ok(())
}

pub fn get_project(name: &str) -> Result<ProjectEntry> {
    let reg = load_registry()?;
    reg.projects
        .into_iter()
        .find(|p| p.name == name)
        .with_context(|| format!("project '{}' not found", name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_path_exists() {
        let path = registry_path().unwrap();
        assert!(path.to_string_lossy().contains(".virgil-cli"));
        assert!(path.to_string_lossy().ends_with("projects.json"));
    }
}
