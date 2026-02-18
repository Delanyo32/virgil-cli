use std::path::Path;

use anyhow::{Context, Result};

use crate::language::Language;
use crate::models::FileMetadata;

pub fn create_parser(language: Language) -> Result<tree_sitter::Parser> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language.tree_sitter_language())
        .context("failed to set tree-sitter language")?;
    Ok(parser)
}

pub fn parse_file(
    parser: &mut tree_sitter::Parser,
    path: &Path,
    root: &Path,
    language: Language,
) -> Result<(FileMetadata, tree_sitter::Tree)> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let tree = parser
        .parse(&source, None)
        .with_context(|| format!("tree-sitter failed to parse {}", path.display()))?;

    let relative_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();

    let size_bytes = source.len() as u64;
    let line_count = source.lines().count() as u64;

    let metadata = FileMetadata {
        path: relative_path,
        name,
        extension,
        language: language.as_str().to_string(),
        size_bytes,
        line_count,
    };

    Ok((metadata, tree))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_parser_all_languages() {
        for lang in Language::all() {
            let parser = create_parser(*lang);
            assert!(parser.is_ok(), "failed to create parser for {lang}");
        }
    }

    #[test]
    fn parse_file_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("hello.ts");
        std::fs::write(&file_path, "const x = 1;\nconst y = 2;\n").unwrap();

        let mut parser = create_parser(Language::TypeScript).unwrap();
        let (meta, _tree) = parse_file(&mut parser, &file_path, dir.path(), Language::TypeScript)
            .expect("parse_file");

        assert_eq!(meta.name, "hello.ts");
        assert_eq!(meta.extension, "ts");
        assert_eq!(meta.language, "typescript");
        assert_eq!(meta.line_count, 2);
        assert_eq!(meta.path, "hello.ts");
    }

    #[test]
    fn parse_file_relative_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let file_path = sub.join("index.ts");
        std::fs::write(&file_path, "export {};\n").unwrap();

        let mut parser = create_parser(Language::TypeScript).unwrap();
        let (meta, _) = parse_file(&mut parser, &file_path, dir.path(), Language::TypeScript)
            .expect("parse_file");

        assert_eq!(meta.path, "src/index.ts");
    }

    #[test]
    fn parse_file_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("empty.ts");
        std::fs::write(&file_path, "").unwrap();

        let mut parser = create_parser(Language::TypeScript).unwrap();
        let (meta, _) = parse_file(&mut parser, &file_path, dir.path(), Language::TypeScript)
            .expect("parse_file");

        assert_eq!(meta.line_count, 0);
        assert_eq!(meta.size_bytes, 0);
    }
}
