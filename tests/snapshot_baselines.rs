//! Per-language baseline snapshot tests (Issues #3–#10).
//!
//! Each test builds a workspace from `../virgil-skills/benchmarks/<lang>/`,
//! populates a fresh in-memory Cozo store via `populate`, and asserts
//! committed row counts against `tests/snapshots/<lang>/baseline.expected`.
//!
//! Each test is skipped (passes vacuously) when the benchmark directory
//! isn't checked out next to this repo — keeps CI green without forcing
//! every contributor to clone the benchmarks sibling.
//!
//! These snapshots pin the **current** Phase-1/2 extractor output. Some
//! per-language gaps are visible (TS/Go doc-attached = 0, PHP calls = 0,
//! C# doc-attached = 2). They're known follow-ups for later phases; the
//! snapshot's job is to catch regressions, not to assert correctness.

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

fn snapshot_for(lang_name: &str, languages: &[Language], bench_rel: &str) {
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
        ("symbol_count", "?[count(s)] := *symbol{id: s}"),
        ("comment_count", "?[count(c)] := *comment{id: c}"),
        ("call_count", "?[count(c)] := *calls{caller_id: c}"),
        (
            "doc_attached_count",
            "?[count(d)] := *comment{documents_id: d, is_doc: true}, d != null",
        ),
        ("file_count", "?[count(f)] := *file{path: f}"),
        ("span_count", "?[count(sp)] := *span{entity_id: sp}"),
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

    let expected_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/snapshots/{lang_name}/baseline.expected"));
    let expected = read_expected(&expected_path);

    for (key, want) in &expected {
        let got = actual.get(key).copied().unwrap_or(0);
        assert_eq!(
            got, *want,
            "[{lang_name}] {key}: expected {want}, got {got}\n\
             Update tests/snapshots/{lang_name}/baseline.expected if the \
             benchmark corpus or extractor changed intentionally."
        );
    }
}

#[test]
fn typescript_baseline() {
    // Cover both .ts/.tsx (nextjs-dashboard) and .js (express-api) — but
    // pin only the TS corpus for now; JS gets its own follow-up snapshot
    // when the JS-specific differences (no type rows) earn one.
    snapshot_for(
        "typescript",
        &[Language::TypeScript, Language::Tsx],
        "typescript/nextjs-dashboard",
    );
}

#[test]
fn python_baseline() {
    snapshot_for("python", &[Language::Python], "python/technical-debt");
}

#[test]
fn go_baseline() {
    snapshot_for("go", &[Language::Go], "go/http-service");
}

#[test]
fn java_baseline() {
    snapshot_for("java", &[Language::Java], "java/spring-api");
}

#[test]
fn php_baseline() {
    snapshot_for("php", &[Language::Php], "php/laravel-store");
}

#[test]
fn c_baseline() {
    snapshot_for("c", &[Language::C], "c/embedded-sensors");
}

#[test]
fn cpp_baseline() {
    snapshot_for("cpp", &[Language::Cpp], "cpp/data-processor");
}

#[test]
fn csharp_baseline() {
    snapshot_for("csharp", &[Language::CSharp], "csharp/dotnet-api");
}
