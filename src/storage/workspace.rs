use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;
use rayon::prelude::*;

use crate::language::Language;
use crate::storage::discovery;
use crate::storage::file_source::{DiskFileSource, FileSource, MemoryFileSource};

pub struct Workspace {
    root: PathBuf,
    source: Box<dyn FileSource>,
    languages: HashMap<String, Language>,
}

impl Workspace {
    /// Discover files, record sizes + languages, return ready-to-use workspace.
    /// File content is read on demand by `DiskFileSource` and cached in a small LRU.
    pub fn load(root: &Path, languages: &[Language], max_file_size: Option<u64>) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("invalid directory: {}", root.display()))?;

        let files = discovery::discover_files(&root, languages)?;

        let discovered: Vec<(String, u64, Language)> = files
            .par_iter()
            .filter_map(|path| {
                let ext = path.extension()?.to_str()?;
                let lang = Language::from_extension(ext)?;

                let size = std::fs::metadata(path).ok()?.len();
                if let Some(max_size) = max_file_size
                    && size > max_size
                {
                    return None;
                }

                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");

                Some((relative, size, lang))
            })
            .collect();

        let mut size_map: HashMap<String, u64> = HashMap::with_capacity(discovered.len());
        let mut lang_map: HashMap<String, Language> = HashMap::with_capacity(discovered.len());
        let mut file_list: Vec<String> = Vec::with_capacity(discovered.len());

        for (rel_path, size, lang) in discovered {
            size_map.insert(rel_path.clone(), size);
            lang_map.insert(rel_path.clone(), lang);
            file_list.push(rel_path);
        }

        let source = Box::new(DiskFileSource::new(root.clone(), file_list, size_map));

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

        info!(bucket = %bucket, prefix = %prefix, "listing S3 objects");
        let keys = crate::storage::s3::list_objects(&location, languages, exclude)?;
        info!(count = keys.len(), "S3 objects matched, starting download");

        let (file_map, size_map) =
            crate::storage::s3::download_objects(&location, &keys, max_file_size)?;
        info!(count = file_map.len(), "S3 download complete");

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

    /// Return a `Workspace` that exposes only the files matching `filter`.
    /// Used by the incremental-refresh path so the builder re-parses just
    /// the touched files. The underlying file content still flows through
    /// the same on-disk LRU.
    pub fn subset<F: FnMut(&str) -> bool>(&self, mut filter: F) -> Workspace {
        let kept: Vec<String> = self
            .source
            .list_files()
            .iter()
            .filter(|p| filter(p.as_str()))
            .cloned()
            .collect();
        let mut sizes: HashMap<String, u64> = HashMap::with_capacity(kept.len());
        let mut langs: HashMap<String, Language> = HashMap::with_capacity(kept.len());
        for p in &kept {
            if let Some(s) = self.source.read_file(p) {
                sizes.insert(p.clone(), s.len() as u64);
            } else {
                sizes.insert(p.clone(), 0);
            }
            if let Some(l) = self.languages.get(p) {
                langs.insert(p.clone(), *l);
            }
        }

        if self.root.exists() {
            let source = Box::new(DiskFileSource::new(self.root.clone(), kept, sizes));
            Workspace {
                root: self.root.clone(),
                source,
                languages: langs,
            }
        } else {
            // S3 workspace — pull file contents into memory for the
            // subset so MemoryFileSource has them.
            let mut files: HashMap<String, Arc<str>> = HashMap::with_capacity(kept.len());
            for p in &kept {
                if let Some(s) = self.source.read_file(p) {
                    files.insert(p.clone(), s);
                }
            }
            let source = Box::new(MemoryFileSource::new(files, sizes));
            Workspace {
                root: self.root.clone(),
                source,
                languages: langs,
            }
        }
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
