use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lru::LruCache;

/// Read-only file source abstraction. Only implementation is
/// [`DiskFileSource`]; the prior `MemoryFileSource` / `S3FileSource`
/// dropped with the S3 + serve scope (see
/// `docs/experiments/duckdb-swap.md`).
pub trait FileSource: Send + Sync {
    /// Read file content by relative path. Returns None if not found.
    fn read_file(&self, relative_path: &str) -> Option<Arc<str>>;

    /// List all available file paths (relative).
    fn list_files(&self) -> &[String];

    /// Check if a file exists.
    fn file_exists(&self, relative_path: &str) -> bool;

    /// Get file size in bytes (without reading content).
    fn file_size(&self, relative_path: &str) -> Option<u64>;
}

/// Default capacity for the disk LRU cache. The working set during a build is
/// approximately one file per rayon worker, so even a small cap stays warm.
const DISK_LRU_CAPACITY: usize = 256;

/// Disk-backed file source with a bounded LRU. Files are read on demand and
/// the cache evicts the least-recently-used entry once it exceeds capacity.
///
/// This avoids retaining ~150-200 MiB of source text in memory for the lifetime
/// of `virgil-cli serve` against a multi-thousand-file workspace.
pub struct DiskFileSource {
    root: PathBuf,
    file_list: Vec<String>,
    sizes: HashMap<String, u64>,
    cache: Mutex<LruCache<String, Arc<str>>>,
}

impl DiskFileSource {
    pub fn new(root: PathBuf, file_list: Vec<String>, sizes: HashMap<String, u64>) -> Self {
        let mut file_list = file_list;
        file_list.sort();
        let cap = NonZeroUsize::new(DISK_LRU_CAPACITY).expect("non-zero capacity");
        Self {
            root,
            file_list,
            sizes,
            cache: Mutex::new(LruCache::new(cap)),
        }
    }
}

impl FileSource for DiskFileSource {
    fn read_file(&self, relative_path: &str) -> Option<Arc<str>> {
        if !self.sizes.contains_key(relative_path) {
            return None;
        }
        if let Some(hit) = self
            .cache
            .lock()
            .ok()
            .and_then(|mut c| c.get(relative_path).cloned())
        {
            return Some(hit);
        }
        let bytes = std::fs::read(self.root.join(relative_path)).ok()?;
        let s: Arc<str> = String::from_utf8(bytes).ok()?.into();
        if let Ok(mut c) = self.cache.lock() {
            c.put(relative_path.to_string(), s.clone());
        }
        Some(s)
    }

    fn list_files(&self) -> &[String] {
        &self.file_list
    }

    fn file_exists(&self, relative_path: &str) -> bool {
        self.sizes.contains_key(relative_path)
    }

    fn file_size(&self, relative_path: &str) -> Option<u64> {
        self.sizes.get(relative_path).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_file_source_reads_from_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn b() {}").unwrap();

        let sizes: HashMap<String, u64> = [("a.rs".to_string(), 9), ("b.rs".to_string(), 9)]
            .into_iter()
            .collect();
        let source = DiskFileSource::new(
            dir.path().to_path_buf(),
            vec!["a.rs".to_string(), "b.rs".to_string()],
            sizes,
        );

        assert_eq!(source.list_files(), &["a.rs", "b.rs"]);
        assert_eq!(source.read_file("a.rs").unwrap().as_ref(), "fn a() {}");
        assert_eq!(source.read_file("b.rs").unwrap().as_ref(), "fn b() {}");
        assert!(source.file_exists("a.rs"));
        assert!(!source.file_exists("missing.rs"));
        assert_eq!(source.file_size("a.rs"), Some(9));
    }

    #[test]
    fn disk_file_source_unknown_file_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = DiskFileSource::new(dir.path().to_path_buf(), vec![], HashMap::new());
        assert!(source.read_file("missing.rs").is_none());
    }

    #[test]
    fn disk_file_source_lru_evicts_under_pressure() {
        // Sanity check: the cache stays bounded and re-reads from disk after
        // eviction. We don't introspect the LRU directly; we just verify that
        // reading 2x the cap doesn't blow up and still returns correct content.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sizes = HashMap::new();
        let mut file_list = Vec::new();
        for i in 0..(DISK_LRU_CAPACITY * 2) {
            let name = format!("f{i}.rs");
            std::fs::write(dir.path().join(&name), format!("// {i}")).unwrap();
            sizes.insert(name.clone(), 6);
            file_list.push(name);
        }

        let source = DiskFileSource::new(dir.path().to_path_buf(), file_list, sizes);
        for i in 0..(DISK_LRU_CAPACITY * 2) {
            let name = format!("f{i}.rs");
            let content = source.read_file(&name).unwrap();
            assert!(content.contains(&format!("{i}")));
        }
    }
}
