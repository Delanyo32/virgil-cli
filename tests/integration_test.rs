use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;

use arrow::array::AsArray;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use virgil_cli::discovery;
use virgil_cli::language::Language;
use virgil_cli::models::{FileMetadata, SymbolInfo, SymbolKind};
use virgil_cli::output;
use virgil_cli::parser;
use virgil_cli::symbols;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Parse a single fixture file and return (metadata, symbols).
fn parse_fixture(filename: &str, language: Language) -> (FileMetadata, Vec<SymbolInfo>) {
    let dir = fixtures_dir();
    let path = dir.join(filename);
    let mut ts_parser = parser::create_parser(language).expect("create parser");
    let (metadata, tree) = parser::parse_file(&mut ts_parser, &path, &dir, language)
        .expect("parse_file");
    let source = std::fs::read_to_string(&path).expect("read source");
    let query = symbols::compile_query(language).expect("compile query");
    let syms = symbols::extract_symbols(&tree, source.as_bytes(), &query, &metadata.path);
    (metadata, syms)
}

#[test]
fn full_pipeline_typescript() {
    let (meta, syms) = parse_fixture("sample.ts", Language::TypeScript);
    assert_eq!(meta.name, "sample.ts");
    assert_eq!(meta.extension, "ts");
    assert_eq!(meta.language, "typescript");

    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(syms.len(), 10, "expected 10 symbols, got: {names:?}");

    // Verify all expected names are present
    let expected = [
        "greet", "UserService", "API_URL", "fetchData", "User",
        "UserId", "Role", "helper", "getName", "internalHandler",
    ];
    for name in &expected {
        assert!(names.contains(name), "missing symbol: {name}");
    }

    // Write to parquet and read back
    let dir = tempfile::tempdir().expect("tempdir");
    output::write_symbols_parquet(&syms, dir.path()).expect("write parquet");

    let file = File::open(dir.path().join("symbols.parquet")).expect("open");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("reader builder")
        .build()
        .expect("reader");
    let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("read batches");
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 10);

    let parquet_names = batch.column(0).as_string::<i32>();
    let read_names: Vec<&str> = (0..batch.num_rows()).map(|i| parquet_names.value(i)).collect();
    for name in &expected {
        assert!(read_names.contains(name), "parquet missing symbol: {name}");
    }
}

#[test]
fn full_pipeline_javascript() {
    let (_meta, syms) = parse_fixture("sample.js", Language::JavaScript);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(syms.len(), 6, "expected 6 symbols, got: {names:?}");

    // JS should have no TS-only kinds
    for sym in &syms {
        assert_ne!(sym.kind, SymbolKind::Interface);
        assert_ne!(sym.kind, SymbolKind::TypeAlias);
        assert_ne!(sym.kind, SymbolKind::Enum);
    }

    let expected = ["add", "Calculator", "multiply", "PI", "square", "legacy"];
    for name in &expected {
        assert!(names.contains(name), "missing symbol: {name}");
    }
}

#[test]
fn full_pipeline_tsx() {
    let (_meta, syms) = parse_fixture("component.tsx", Language::Tsx);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(syms.len(), 3, "expected 3 symbols, got: {names:?}");
    assert!(names.contains(&"App"));
    assert!(names.contains(&"Header"));
    assert!(names.contains(&"Props"));
}

#[test]
fn full_pipeline_jsx() {
    let (_meta, syms) = parse_fixture("component.jsx", Language::Jsx);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(syms.len(), 2, "expected 2 symbols, got: {names:?}");
    assert!(names.contains(&"Button"));
    assert!(names.contains(&"styles"));
}

#[test]
fn full_pipeline_empty_file() {
    let (meta, syms) = parse_fixture("empty.ts", Language::TypeScript);
    assert_eq!(syms.len(), 0);
    assert_eq!(meta.line_count, 0);
}

#[test]
fn discover_fixtures() {
    let dir = fixtures_dir();
    let files = discovery::discover_files(&dir, Language::all()).expect("discover");
    assert_eq!(files.len(), 5, "expected 5 fixture files, got: {files:?}");
}

#[test]
fn parquet_preserves_export_flag() {
    let (_meta, syms) = parse_fixture("sample.ts", Language::TypeScript);

    let dir = tempfile::tempdir().expect("tempdir");
    output::write_symbols_parquet(&syms, dir.path()).expect("write parquet");

    let file = File::open(dir.path().join("symbols.parquet")).expect("open");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("reader builder")
        .build()
        .expect("reader");
    let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("read batches");
    let batch = &batches[0];

    let names_col = batch.column(0).as_string::<i32>();
    let exported_col = batch.column(7).as_boolean();

    // Build map of name -> is_exported from parquet
    let mut export_map: HashMap<String, bool> = HashMap::new();
    for i in 0..batch.num_rows() {
        export_map.insert(names_col.value(i).to_string(), exported_col.value(i));
    }

    // Exported symbols
    for name in &["greet", "UserService", "API_URL", "fetchData", "User", "UserId", "Role"] {
        assert_eq!(export_map.get(*name), Some(&true), "{name} should be exported");
    }

    // Non-exported symbols
    for name in &["helper", "getName", "internalHandler"] {
        assert_eq!(export_map.get(*name), Some(&false), "{name} should not be exported");
    }
}
