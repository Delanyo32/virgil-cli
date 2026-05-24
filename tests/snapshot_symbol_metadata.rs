//! Per-language symbol-metadata snapshot tests (Issue #12).
//!
//! For each language, asserts counts of symbols carrying each visibility
//! value, a non-trivial qualified_name, a non-null parent_id, and each
//! of the four flag columns. Aligns with the acceptance criteria for
//! issue #12.
//!
//! Tests skip vacuously when the benchmark sibling repo isn't checked
//! out, mirroring snapshot_baselines.rs and snapshot_locals.rs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use virgil_cli::cozo::{CozoStore, populate};
use virgil_cli::graph::builder::GraphBuilder;
use virgil_cli::language::Language;
use virgil_cli::storage::workspace::Workspace;

fn benchmark_dir(rel: &str) -> Option<PathBuf> {
    for prefix in [
        "../virgil-skills/benchmarks/",
        "../../virgil-skills/benchmarks/",
        "/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/",
    ] {
        let p = PathBuf::from(format!("{prefix}{rel}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn read_expected(path: &Path) -> BTreeMap<String, i64> {
    let text = std::fs::read_to_string(path).expect("read expected");
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v: i64 = v.trim().parse().expect("parse i64");
        out.insert(k.trim().to_string(), v);
    }
    out
}

fn snapshot_symbol_metadata_for(lang_name: &str, languages: &[Language], bench_rel: &str) {
    let Some(bench) = benchmark_dir(bench_rel) else {
        eprintln!("skipping {lang_name}: benchmark {bench_rel} not found");
        return;
    };

    let ws = Workspace::load(&bench, languages, None).expect("load workspace");
    let store = CozoStore::open_in_memory().expect("open store");
    let graph = GraphBuilder::new(&ws, languages)
        .build(&store)
        .expect("build graph");
    populate(&store, &graph, Some(&ws)).expect("populate");

    let counts = [
        (
            "public_count",
            "?[count(s)] := *symbol{id: s, visibility: \"public\"}",
        ),
        (
            "private_count",
            "?[count(s)] := *symbol{id: s, visibility: \"private\"}",
        ),
        (
            "internal_count",
            "?[count(s)] := *symbol{id: s, visibility: \"internal\"}",
        ),
        (
            "protected_count",
            "?[count(s)] := *symbol{id: s, visibility: \"protected\"}",
        ),
        (
            "qualified_name_count",
            "?[count(s)] := *symbol{id: s, name: n, qualified_name: q}, n != q",
        ),
        (
            "parent_id_count",
            "?[count(s)] := *symbol{id: s, parent_id: p}, p != null",
        ),
        (
            "is_async_count",
            "?[count(s)] := *symbol{id: s, is_async: true}",
        ),
        (
            "is_static_count",
            "?[count(s)] := *symbol{id: s, is_static: true}",
        ),
        (
            "is_abstract_count",
            "?[count(s)] := *symbol{id: s, is_abstract: true}",
        ),
        (
            "is_mutable_count",
            "?[count(s)] := *symbol{id: s, is_mutable: true}",
        ),
    ];

    let mut actual: BTreeMap<String, i64> = BTreeMap::new();
    for (key, q) in counts {
        let rows = store.run_query(q, BTreeMap::new()).expect("query");
        let n = rows
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(|v| match v {
                cozo::DataValue::Num(cozo::Num::Int(i)) => Some(*i),
                _ => None,
            })
            .unwrap_or(0);
        actual.insert(key.to_string(), n);
    }

    let expected_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/snapshots/{lang_name}/symbol-metadata.expected"
    ));
    let expected = read_expected(&expected_path);

    for (key, want) in &expected {
        let got = actual.get(key).copied().unwrap_or(0);
        assert_eq!(
            got, *want,
            "[{lang_name}] {key}: expected {want}, got {got}\n\
             Update tests/snapshots/{lang_name}/symbol-metadata.expected if \
             the benchmark corpus or extractor changed intentionally."
        );
    }
}

#[test]
fn rust_symbol_metadata() {
    snapshot_symbol_metadata_for("rust", &[Language::Rust], "rust/systems-cli");
}

#[test]
fn typescript_symbol_metadata() {
    snapshot_symbol_metadata_for(
        "typescript",
        &[Language::TypeScript, Language::Tsx],
        "typescript/nextjs-dashboard",
    );
}

#[test]
fn python_symbol_metadata() {
    snapshot_symbol_metadata_for("python", &[Language::Python], "python/technical-debt");
}

#[test]
fn go_symbol_metadata() {
    snapshot_symbol_metadata_for("go", &[Language::Go], "go/http-service");
}

#[test]
fn java_symbol_metadata() {
    snapshot_symbol_metadata_for("java", &[Language::Java], "java/spring-api");
}

#[test]
fn php_symbol_metadata() {
    snapshot_symbol_metadata_for("php", &[Language::Php], "php/laravel-store");
}

#[test]
fn c_symbol_metadata() {
    snapshot_symbol_metadata_for("c", &[Language::C], "c/embedded-sensors");
}

#[test]
fn cpp_symbol_metadata() {
    snapshot_symbol_metadata_for("cpp", &[Language::Cpp], "cpp/data-processor");
}

#[test]
fn csharp_symbol_metadata() {
    snapshot_symbol_metadata_for("csharp", &[Language::CSharp], "csharp/dotnet-api");
}
