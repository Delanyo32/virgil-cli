use std::path::PathBuf;

use virgil_cli::language::Language;
use virgil_cli::languages;
use virgil_cli::models::{ImportInfo, SymbolInfo, SymbolKind};
use virgil_cli::parser;
use virgil_cli::storage::discovery;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Parse a single fixture file and return (metadata, symbols, imports).
fn parse_fixture_full(
    filename: &str,
    language: Language,
) -> (
    virgil_cli::models::FileMetadata,
    Vec<SymbolInfo>,
    Vec<ImportInfo>,
) {
    let dir = fixtures_dir();
    let path = dir.join(filename);
    let mut ts_parser = parser::create_parser(language).expect("create parser");
    let (metadata, tree) =
        parser::parse_file(&mut ts_parser, &path, &dir, language).expect("parse_file");
    let source = std::fs::read_to_string(&path).expect("read source");
    let sym_query = languages::compile_symbol_query(language).expect("compile query");
    let syms = languages::extract_symbols(
        &tree,
        source.as_bytes(),
        &sym_query,
        &metadata.path,
        language,
    );
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
fn parse_fixture(
    filename: &str,
    language: Language,
) -> (virgil_cli::models::FileMetadata, Vec<SymbolInfo>) {
    let dir = fixtures_dir();
    let path = dir.join(filename);
    let mut ts_parser = parser::create_parser(language).expect("create parser");
    let (metadata, tree) =
        parser::parse_file(&mut ts_parser, &path, &dir, language).expect("parse_file");
    let source = std::fs::read_to_string(&path).expect("read source");
    let query = languages::compile_symbol_query(language).expect("compile query");
    let syms =
        languages::extract_symbols(&tree, source.as_bytes(), &query, &metadata.path, language);
    (metadata, syms)
}

#[test]
fn full_pipeline_typescript() {
    let (meta, syms) = parse_fixture("sample.ts", Language::TypeScript);
    assert_eq!(meta.name, "sample.ts");
    assert_eq!(meta.extension, "ts");
    assert_eq!(meta.language, "typescript");

    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    // Issue #11: parameter symbols (name, url) now extracted as well.
    // Issue #18.1: class fields (id, name) emitted as Field symbols.
    assert_eq!(syms.len(), 14, "expected 14 symbols, got: {names:?}");

    let expected = [
        "greet",
        "UserService",
        "API_URL",
        "fetchData",
        "User",
        "UserId",
        "Role",
        "helper",
        "getName",
        "internalHandler",
        "name",
        "url",
    ];
    for name in &expected {
        assert!(names.contains(name), "missing symbol: {name}");
    }
}

#[test]
fn full_pipeline_javascript() {
    let (_meta, syms) = parse_fixture("sample.js", Language::JavaScript);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    // Issue #11: parameter symbols (a, b for add/multiply; x for square) now
    // extracted too — 6 declared + 5 parameters = 11.
    assert_eq!(syms.len(), 11, "expected 11 symbols, got: {names:?}");

    for sym in &syms {
        assert_ne!(sym.kind, SymbolKind::Interface);
        assert_ne!(sym.kind, SymbolKind::TypeAlias);
        assert_ne!(sym.kind, SymbolKind::Enum);
    }

    let expected = [
        "add",
        "Calculator",
        "multiply",
        "PI",
        "square",
        "legacy",
        "a",
        "b",
        "x",
    ];
    for name in &expected {
        assert!(names.contains(name), "missing symbol: {name}");
    }
}

#[test]
fn full_pipeline_tsx() {
    let (_meta, syms) = parse_fixture("component.tsx", Language::Tsx);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    // Issue #11: parameter symbol `props` (Header's prop arg) now extracted.
    // Issue #18.1: interface property `title` emitted as Field symbol.
    assert_eq!(syms.len(), 5, "expected 5 symbols, got: {names:?}");
    assert!(names.contains(&"App"));
    assert!(names.contains(&"Header"));
    assert!(names.contains(&"Props"));
    assert!(names.contains(&"props"));
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
fn import_extraction_typescript() {
    let (_meta, _syms, imps) = parse_fixture_full("imports_sample.ts", Language::TypeScript);

    let static_count = imps.iter().filter(|i| i.kind == "static").count();
    let dynamic_count = imps.iter().filter(|i| i.kind == "dynamic").count();
    let reexport_count = imps.iter().filter(|i| i.kind == "re_export").count();

    // One row per import statement (not per binding): 7 static + 1 dynamic
    // + 2 re-exports in imports_sample.ts.
    assert_eq!(static_count, 7, "expected 7 static imports");
    assert_eq!(dynamic_count, 1, "expected 1 dynamic import");
    assert_eq!(reexport_count, 2, "expected 2 re-exports");

    let react = imps.iter().filter(|i| i.module_specifier == "react").count();
    assert_eq!(react, 2, "expected 2 statements importing from react");

    let namespace = imps.iter().find(|i| i.module_specifier == "path");
    assert!(namespace.is_some(), "missing namespace import for path");

    let type_only = imps.iter().find(|i| i.module_specifier == "./models");
    assert!(type_only.is_some(), "missing type-only import from ./models");

    let side_effect = imps.iter().find(|i| i.module_specifier == "./polyfill");
    assert!(
        side_effect.is_some(),
        "missing side-effect import ./polyfill"
    );

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

    assert!(
        static_count >= 2,
        "expected at least 2 static imports, got {static_count}"
    );
    assert_eq!(require_count, 2, "expected 2 require calls");
    assert_eq!(dynamic_count, 1, "expected 1 dynamic import");
}

#[test]
fn file_content_is_stored() {
    let store = virgil_cli::db::DbStore::open_in_memory().expect("open store");
    let workspace =
        virgil_cli::storage::workspace::Workspace::load(&fixtures_dir(), Language::all(), None)
            .expect("load workspace");
    virgil_cli::graph::builder::GraphBuilder::new(&workspace, Language::all())
        .build(&store)
        .expect("build");

    let rows = store
        .run_query(
            "SELECT content FROM file WHERE content <> '' LIMIT 1",
            Default::default(),
        )
        .expect("query");
    assert_eq!(
        rows.rows.len(),
        1,
        "expected at least one file with stored content"
    );

    // The stored text is the real source, not a placeholder: byte
    // length must match the fixture on disk.
    let expected_bytes = std::fs::read_to_string(fixtures_dir().join("sample.ts"))
        .expect("read fixture")
        .len();
    let matched = store
        .run_query(
            &format!(
                "SELECT path FROM file \
                 WHERE path = 'sample.ts' AND strlen(content) = {expected_bytes}"
            ),
            Default::default(),
        )
        .expect("query");
    assert_eq!(matched.rows.len(), 1, "sample.ts content differs from disk");
}
