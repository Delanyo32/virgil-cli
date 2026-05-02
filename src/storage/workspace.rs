use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::storage::discovery;
use crate::storage::file_source::{FileSource, MemoryFileSource};
use crate::language::Language;

pub struct Workspace {
    root: PathBuf,
    source: Box<dyn FileSource>,
    languages: HashMap<String, Language>,
}

impl Workspace {
    /// Discover files, load into memory, return ready-to-use workspace.
    pub fn load(root: &Path, languages: &[Language], max_file_size: Option<u64>) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("invalid directory: {}", root.display()))?;

        let files = discovery::discover_files(&root, languages)?;

        let loaded: Vec<(String, Arc<str>, Language)> = files
            .par_iter()
            .filter_map(|path| {
                let ext = path.extension()?.to_str()?;
                let lang = Language::from_extension(ext)?;

                if let Some(max_size) = max_file_size
                    && let Ok(meta) = std::fs::metadata(path)
                    && meta.len() > max_size
                {
                    return None;
                }

                let content = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Warning: failed to read {}: {e}", path.display());
                        return None;
                    }
                };

                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");

                Some((relative, Arc::from(content.as_str()), lang))
            })
            .collect();

        let mut file_map: HashMap<String, Arc<str>> = HashMap::with_capacity(loaded.len());
        let mut size_map: HashMap<String, u64> = HashMap::with_capacity(loaded.len());
        let mut lang_map: HashMap<String, Language> = HashMap::with_capacity(loaded.len());

        for (rel_path, content, lang) in loaded {
            let size = content.len() as u64;
            size_map.insert(rel_path.clone(), size);
            lang_map.insert(rel_path.clone(), lang);
            file_map.insert(rel_path, content);
        }

        let source = Box::new(MemoryFileSource::new(file_map, size_map));

        Ok(Self {
            root,
            source,
            languages: lang_map,
        })
    }

    /// Load files from S3 into memory, return ready-to-use workspace.
    pub fn load_from_s3(
        bucket: &str,
        prefix: &str,
        languages: &[Language],
        exclude: &[String],
        max_file_size: Option<u64>,
    ) -> Result<Self> {
        let location = crate::storage::s3::S3Location {
            bucket: bucket.to_string(),
            prefix: prefix.to_string(),
        };

        eprintln!("Listing objects in s3://{bucket}/{prefix}...");
        let keys = crate::storage::s3::list_objects(&location, languages, exclude)?;
        eprintln!("Found {} files, downloading...", keys.len());

        let (file_map, size_map) = crate::storage::s3::download_objects(&location, &keys, max_file_size)?;
        eprintln!("Downloaded {} files into memory", file_map.len());

        // Build language map from extensions
        let mut lang_map: HashMap<String, Language> = HashMap::with_capacity(file_map.len());
        for key in file_map.keys() {
            if let Some(ext) = std::path::Path::new(key)
                .extension()
                .and_then(|e| e.to_str())
                && let Some(lang) = Language::from_extension(ext)
            {
                lang_map.insert(key.clone(), lang);
            }
        }

        let source = Box::new(MemoryFileSource::new(file_map, size_map));

        // Use a synthetic root path for S3 workspaces
        let root = PathBuf::from(format!("s3://{bucket}/{prefix}"));

        Ok(Self {
            root,
            source,
            languages: lang_map,
        })
    }

    /// Read file content by relative path.
    pub fn read_file(&self, relative_path: &str) -> Option<Arc<str>> {
        self.source.read_file(relative_path)
    }

    /// Get language for a loaded file.
    pub fn file_language(&self, relative_path: &str) -> Option<Language> {
        self.languages.get(relative_path).copied()
    }

    /// All loaded relative paths (sorted).
    pub fn files(&self) -> &[String] {
        self.source.list_files()
    }

    /// Project root on disk.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Total files loaded.
    pub fn file_count(&self) -> usize {
        self.source.list_files().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_load_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = Workspace::load(dir.path(), &[Language::Rust], None).unwrap();
        assert_eq!(ws.file_count(), 0);
        assert!(ws.files().is_empty());
    }

    #[test]
    fn workspace_load_with_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub mod foo;").unwrap();

        let ws = Workspace::load(dir.path(), &[Language::Rust], None).unwrap();
        assert_eq!(ws.file_count(), 2);
        assert!(ws.read_file("main.rs").is_some());
        assert!(ws.read_file("lib.rs").is_some());
        assert_eq!(ws.file_language("main.rs"), Some(Language::Rust));
    }

    #[test]
    fn workspace_filters_by_language() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

        let ws = Workspace::load(dir.path(), &[Language::Rust], None).unwrap();
        assert_eq!(ws.file_count(), 1);
        assert!(ws.read_file("main.rs").is_some());
        assert!(ws.read_file("app.ts").is_none());
    }

    #[test]
    fn workspace_max_file_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small.rs"), "fn x() {}").unwrap();
        std::fs::write(dir.path().join("big.rs"), "x".repeat(1000)).unwrap();

        let ws = Workspace::load(dir.path(), &[Language::Rust], Some(500)).unwrap();
        assert!(ws.read_file("small.rs").is_some());
        assert!(ws.read_file("big.rs").is_none());
    }

    #[test]
    fn workspace_subdirectories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("lib.rs"), "pub fn hello() {}").unwrap();

        let ws = Workspace::load(dir.path(), &[Language::Rust], None).unwrap();
        assert_eq!(ws.file_count(), 1);
        assert!(ws.read_file("src/lib.rs").is_some());
    }
}
