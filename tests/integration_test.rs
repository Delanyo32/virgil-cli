use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;

use arrow::array::AsArray;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use virgil_cli::discovery;
use virgil_cli::language::Language;
use virgil_cli::languages;
use virgil_cli::models::{FileMetadata, ImportInfo, SymbolInfo, SymbolKind};
use virgil_cli::output;
use virgil_cli::parser;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Parse a single fixture file and return (metadata, symbols, imports).
fn parse_fixture_full(
    filename: &str,
    language: Language,
) -> (FileMetadata, Vec<SymbolInfo>, Vec<ImportInfo>) {
    let dir = fixtures_dir();
    let path = dir.join(filename);
    let mut ts_parser = parser::create_parser(language).expect("create parser");
    let (metadata, tree) =
        parser::parse_file(&mut ts_parser, &path, &dir, language).expect("parse_file");
    let source = std::fs::read_to_string(&path).expect("read source");
    let sym_query = languages::compile_symbol_query(language).expect("compile query");
    let syms = languages::extract_symbols(&tree, source.as_bytes(), &sym_query, &metadata.path, language);
    let imp_query = languages::compile_import_query(language).expect("compile import query");
    let imps = languages::extract_imports(
        &tree,
        source.as_bytes(),
        &imp_query,
        &metadata.path,
        language,
    );
    (metadata, syms, imps)
}

/// Parse a single fixture file and return (metadata, symbols).
fn parse_fixture(filename: &str, language: Language) -> (FileMetadata, Vec<SymbolInfo>) {
    let dir = fixtures_dir();
    let path = dir.join(filename);
    let mut ts_parser = parser::create_parser(language).expect("create parser");
    let (metadata, tree) = parser::parse_file(&mut ts_parser, &path, &dir, language)
        .expect("parse_file");
    let source = std::fs::read_to_string(&path).expect("read source");
    let query = languages::compile_symbol_query(language).expect("compile query");
    let syms = languages::extract_symbols(&tree, source.as_bytes(), &query, &metadata.path, language);
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
    assert_eq!(files.len(), 7, "expected 7 fixture files, got: {files:?}");
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

#[test]
fn import_extraction_typescript() {
    let (_meta, _syms, imps) = parse_fixture_full("imports_sample.ts", Language::TypeScript);

    // Count by kind
    let static_count = imps.iter().filter(|i| i.kind == "static").count();
    let dynamic_count = imps.iter().filter(|i| i.kind == "dynamic").count();
    let reexport_count = imps.iter().filter(|i| i.kind == "re_export").count();

    assert!(static_count >= 8, "expected at least 8 static imports, got {static_count}");
    assert_eq!(dynamic_count, 1, "expected 1 dynamic import");
    assert!(reexport_count >= 2, "expected at least 2 re-exports, got {reexport_count}");

    // Check specific imports
    let react_default = imps
        .iter()
        .find(|i| i.module_specifier == "react" && i.imported_name == "default");
    assert!(react_default.is_some(), "missing default import from react");
    assert_eq!(react_default.unwrap().local_name, "React");

    let namespace = imps.iter().find(|i| i.imported_name == "*" && i.local_name == "path");
    assert!(namespace.is_some(), "missing namespace import for path");

    let aliased = imps
        .iter()
        .find(|i| i.imported_name == "useState" && i.local_name == "useMyState");
    assert!(aliased.is_some(), "missing aliased import useState as useMyState");

    let type_only = imps
        .iter()
        .find(|i| i.imported_name == "User" && i.module_specifier == "./models");
    assert!(type_only.is_some(), "missing type-only import User");
    assert!(type_only.unwrap().is_type_only, "User import should be type-only");

    let side_effect = imps
        .iter()
        .find(|i| i.module_specifier == "./polyfill");
    assert!(side_effect.is_some(), "missing side-effect import ./polyfill");

    let dynamic = imps.iter().find(|i| i.kind == "dynamic");
    assert!(dynamic.is_some(), "missing dynamic import");
    assert_eq!(dynamic.unwrap().module_specifier, "./lazy-component");
}

#[test]
fn import_extraction_javascript() {
    let (_meta, _syms, imps) = parse_fixture_full("imports_sample.js", Language::JavaScript);

    let static_count = imps.iter().filter(|i| i.kind == "static").count();
    let require_count = imps.iter().filter(|i| i.kind == "require").count();
    let dynamic_count = imps.iter().filter(|i| i.kind == "dynamic").count();

    assert!(static_count >= 2, "expected at least 2 static imports, got {static_count}");
    assert_eq!(require_count, 2, "expected 2 require calls");
    assert_eq!(dynamic_count, 1, "expected 1 dynamic import");
}

#[test]
fn imports_parquet_round_trip() {
    let (_meta, _syms, imps) = parse_fixture_full("imports_sample.ts", Language::TypeScript);
    assert!(!imps.is_empty(), "should have extracted imports");

    let dir = tempfile::tempdir().expect("tempdir");
    output::write_imports_parquet(&imps, dir.path()).expect("write imports parquet");

    let path = dir.path().join("imports.parquet");
    assert!(path.exists());

    let file = File::open(&path).expect("open");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("reader builder")
        .build()
        .expect("reader");
    let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("read batches");
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), imps.len());

    // Verify source_file column
    let source_files = batch.column(0).as_string::<i32>();
    for i in 0..batch.num_rows() {
        assert_eq!(source_files.value(i), "imports_sample.ts");
    }

    // Verify module_specifier column has expected values
    let specs = batch.column(1).as_string::<i32>();
    let spec_values: Vec<&str> = (0..batch.num_rows()).map(|i| specs.value(i)).collect();
    assert!(spec_values.contains(&"react"), "parquet missing 'react' module specifier");
    assert!(spec_values.contains(&"./utils"), "parquet missing './utils' module specifier");
}
