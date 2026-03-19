use std::collections::HashMap;
use std::sync::Arc;

/// Read-only file source abstraction.
/// Implementations: MemoryFileSource (local), future S3FileSource.
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

/// In-memory file source backed by a HashMap.
/// After construction, all reads are zero-copy via Arc<str>.
pub struct MemoryFileSource {
    files: HashMap<String, Arc<str>>,
    file_list: Vec<String>,
    sizes: HashMap<String, u64>,
}

impl MemoryFileSource {
    pub fn new(files: HashMap<String, Arc<str>>, sizes: HashMap<String, u64>) -> Self {
        let mut file_list: Vec<String> = files.keys().cloned().collect();
        file_list.sort();
        Self {
            files,
            file_list,
            sizes,
        }
    }
}

impl FileSource for MemoryFileSource {
    fn read_file(&self, relative_path: &str) -> Option<Arc<str>> {
        self.files.get(relative_path).cloned()
    }

    fn list_files(&self) -> &[String] {
        &self.file_list
    }

    fn file_exists(&self, relative_path: &str) -> bool {
        self.files.contains_key(relative_path)
    }

    fn file_size(&self, relative_path: &str) -> Option<u64> {
        self.sizes.get(relative_path).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_file_source_basic() {
        let mut files = HashMap::new();
        files.insert("src/main.rs".to_string(), Arc::from("fn main() {}"));
        files.insert("src/lib.rs".to_string(), Arc::from("pub mod foo;"));

        let mut sizes = HashMap::new();
        sizes.insert("src/main.rs".to_string(), 12);
        sizes.insert("src/lib.rs".to_string(), 12);

        let source = MemoryFileSource::new(files, sizes);

        assert_eq!(source.list_files().len(), 2);
        assert!(source.file_exists("src/main.rs"));
        assert!(!source.file_exists("src/missing.rs"));
        assert_eq!(
            source.read_file("src/main.rs").unwrap().as_ref(),
            "fn main() {}"
        );
        assert_eq!(source.file_size("src/main.rs"), Some(12));
        assert_eq!(source.file_size("missing"), None);
    }

    #[test]
    fn file_list_is_sorted() {
        let mut files = HashMap::new();
        files.insert("z.rs".to_string(), Arc::from(""));
        files.insert("a.rs".to_string(), Arc::from(""));
        files.insert("m.rs".to_string(), Arc::from(""));

        let source = MemoryFileSource::new(files, HashMap::new());
        let list = source.list_files();
        assert_eq!(list, &["a.rs", "m.rs", "z.rs"]);
    }
}
