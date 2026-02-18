use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

use crate::language::Language;

pub fn discover_files(root: &Path, languages: &[Language]) -> Result<Vec<PathBuf>> {
    let extensions: Vec<&str> = languages.iter().map(|l| l.extension()).collect();

    let mut files = Vec::new();
    for entry in WalkBuilder::new(root).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if extensions.contains(&ext) {
                files.push(path.to_path_buf());
            }
        }
    }

    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.ts"), "const x = 1;").unwrap();
        std::fs::write(dir.path().join("util.js"), "const y = 2;").unwrap();
        std::fs::write(dir.path().join("style.css"), "body {}").unwrap();
        dir
    }

    #[test]
    fn discover_single_language() {
        let dir = create_test_dir();
        let files = discover_files(dir.path(), &[Language::TypeScript]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("main.ts"));
    }

    #[test]
    fn discover_multiple_languages() {
        let dir = create_test_dir();
        let files = discover_files(dir.path(), &[Language::TypeScript, Language::JavaScript]).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn discover_ignores_non_matching_extensions() {
        let dir = create_test_dir();
        let files = discover_files(dir.path(), &[Language::Tsx]).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn discover_subdirectories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("index.ts"), "export {};").unwrap();
        std::fs::write(dir.path().join("root.ts"), "const x = 1;").unwrap();

        let files = discover_files(dir.path(), &[Language::TypeScript]).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn discover_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = discover_files(dir.path(), &[Language::TypeScript]).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn discover_respects_gitignore() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Initialize a git repo so .gitignore is respected
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("git init");

        std::fs::write(dir.path().join(".gitignore"), "ignored/\n").unwrap();
        let ignored = dir.path().join("ignored");
        std::fs::create_dir_all(&ignored).unwrap();
        std::fs::write(ignored.join("skip.ts"), "const x = 1;").unwrap();
        std::fs::write(dir.path().join("keep.ts"), "const y = 2;").unwrap();

        let files = discover_files(dir.path(), &[Language::TypeScript]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("keep.ts"));
    }

    #[test]
    fn discover_results_are_sorted() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("z.ts"), "").unwrap();
        std::fs::write(dir.path().join("a.ts"), "").unwrap();
        std::fs::write(dir.path().join("m.ts"), "").unwrap();

        let files = discover_files(dir.path(), &[Language::TypeScript]).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap().to_str().unwrap()).collect();
        assert_eq!(names, vec!["a.ts", "m.ts", "z.ts"]);
    }
}
