//! Per-language locals snapshot tests (Issue #11 Phase 2 acceptance #5).
//!
//! Each test builds a workspace from `../virgil-skills/benchmarks/<lang>/`,
//! populates a fresh in-memory Cozo store, and asserts the count of
//! `parameter`- and `variable`-kinded symbol rows against
//! `tests/snapshots/<lang>/locals.expected`.
//!
//! The matching `tests/snapshots/<lang>/locals.cozoql` file is human-
//! readable documentation of the Cozoscript shape these counts come from;
//! the runner uses inline queries for the same reason `baseline.cozoql`
//! is documentation-only — keeps the test self-contained while letting the
//! query file serve as a spec.
//!
//! Tests skip vacuously when the benchmark sibling repo isn't checked out.

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

fn snapshot_locals_for(lang_name: &str, languages: &[Language], bench_rel: &str) {
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
            "parameter_count",
            "?[count(s)] := *symbol{id: s, kind: \"parameter\"}",
        ),
        (
            "local_count",
            "?[count(s)] := *symbol{id: s, kind: \"variable\"}",
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

    let expected_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/snapshots/{lang_name}/locals.expected"));
    let expected = read_expected(&expected_path);

    for (key, want) in &expected {
        let got = actual.get(key).copied().unwrap_or(0);
        assert_eq!(
            got, *want,
            "[{lang_name}] {key}: expected {want}, got {got}\n\
             Update tests/snapshots/{lang_name}/locals.expected if the \
             benchmark corpus or extractor changed intentionally."
        );
    }
}

#[test]
fn rust_locals() {
    snapshot_locals_for("rust", &[Language::Rust], "rust/systems-cli");
}

#[test]
fn typescript_locals() {
    snapshot_locals_for(
        "typescript",
        &[Language::TypeScript, Language::Tsx],
        "typescript/nextjs-dashboard",
    );
}

#[test]
fn python_locals() {
    snapshot_locals_for("python", &[Language::Python], "python/technical-debt");
}

#[test]
fn go_locals() {
    snapshot_locals_for("go", &[Language::Go], "go/http-service");
}

#[test]
fn java_locals() {
    snapshot_locals_for("java", &[Language::Java], "java/spring-api");
}

#[test]
fn php_locals() {
    snapshot_locals_for("php", &[Language::Php], "php/laravel-store");
}

#[test]
fn c_locals() {
    snapshot_locals_for("c", &[Language::C], "c/embedded-sensors");
}

#[test]
fn cpp_locals() {
    snapshot_locals_for("cpp", &[Language::Cpp], "cpp/data-processor");
}

#[test]
fn csharp_locals() {
    snapshot_locals_for("csharp", &[Language::CSharp], "csharp/dotnet-api");
}
