//! Rust baseline snapshot test (Issue #2).
//!
//! Builds the `../virgil-skills/benchmarks/rust/systems-cli/` workspace,
//! runs the committed Cozoscript at `tests/snapshots/rust/baseline.cozoql`,
//! and compares the output against `tests/snapshots/rust/baseline.expected`.
//!
//! The test is skipped (passes vacuously) when the benchmark directory
//! isn't checked out next to this repo — keeps CI green without forcing
//! every contributor to clone the benchmarks sibling.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use virgil_cli::cozo::{CozoStore, populate};
use virgil_cli::graph::builder::GraphBuilder;
use virgil_cli::language::Language;
use virgil_cli::storage::workspace::Workspace;

fn benchmark_dir() -> Option<PathBuf> {
    let candidates = [
        // Relative to the worktree root (this file's typical layout).
        "../virgil-skills/benchmarks/rust/systems-cli",
        // Relative to the *main* repo when run from there.
        "../../virgil-skills/benchmarks/rust/systems-cli",
        // Absolute path: useful when invoked from anywhere.
        "/Users/delanyoaborchie/Documents/github/virgil-skills/benchmarks/rust/systems-cli",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
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

#[test]
fn rust_baseline_snapshot_matches() {
    let Some(bench) = benchmark_dir() else {
        eprintln!("skipping: virgil-skills benchmarks/rust/systems-cli not found");
        return;
    };

    let ws = Workspace::load(&bench, &[Language::Rust], None).expect("load workspace");
    let store = CozoStore::open_in_memory().expect("open store");
    let graph = GraphBuilder::new(&ws, &[Language::Rust])
        .build(&store)
        .expect("build graph");
    populate(&store, &graph, Some(&ws)).expect("populate");

    // Run individual count queries; the combined cozoscript in baseline.cozoql
    // is documentary — split here for clearer failure messages.
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

    let expected_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/rust/baseline.expected");
    let expected = read_expected(&expected_path);

    for (key, want) in &expected {
        let got = actual.get(key).copied().unwrap_or(0);
        assert_eq!(
            got, *want,
            "{key}: expected {want}, got {got}\n\
             Update tests/snapshots/rust/baseline.expected if the benchmark \
             corpus changed intentionally."
        );
    }
}
