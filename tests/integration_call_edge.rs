//! Confirms the schema v9 migration: *call_edge is populated at build
//! time with both intra-file and cross-file resolutions, and *call_site
//! is unchanged.

use std::collections::BTreeMap;

use tempfile::tempdir;

use virgil_cli::cozo::{populate, CozoStore};
use virgil_cli::graph::builder::GraphBuilder;
use virgil_cli::language::Language;
use virgil_cli::storage::workspace::Workspace;

#[test]
fn call_edge_is_populated_with_intra_and_cross_file_edges() {
    let dir = tempdir().expect("tempdir");

    // File a.rs defines beta (exported) + alpha (alpha calls beta — intra-file).
    // File b.rs imports beta from a via self::a::beta and calls it (cross-file).
    //
    // NOTE: `use crate::a::beta` would fail here because the Rust extractor's
    // resolve_import prepends "src/" for crate:: paths, resolving to "src/a.rs"
    // which doesn't exist in a flat tempdir. Using `self::a::beta` instead:
    // the self:: resolver strips the last segment ("beta") to get module path
    // "self::a", then resolves relative to b.rs's directory (the tempdir root)
    // yielding "a.rs" — which IS present in known_files.
    std::fs::write(
        dir.path().join("a.rs"),
        "pub fn beta() {}\nfn alpha() { beta(); }\n",
    )
    .expect("write a.rs");
    std::fs::write(
        dir.path().join("b.rs"),
        "use self::a::beta;\nfn gamma() { beta(); }\n",
    )
    .expect("write b.rs");

    let workspace =
        Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
    let store = CozoStore::open_in_memory().expect("open store");
    let graph = GraphBuilder::new(&workspace, &[Language::Rust])
        .build(&store)
        .expect("build graph");
    populate(&store, &graph, Some(&workspace)).expect("populate");

    let edges = store
        .run_query(
            "?[caller, callee, file] := \
             *call_edge{caller_id, callee_id, file_path: file}, \
             *symbol{id: caller_id, name: caller}, \
             *symbol{id: callee_id, name: callee}",
            BTreeMap::new(),
        )
        .expect("call_edge query");

    let pairs: Vec<(String, String)> = edges
        .rows
        .iter()
        .map(|r| {
            (
                match &r[0] {
                    cozo::DataValue::Str(s) => s.to_string(),
                    _ => String::new(),
                },
                match &r[1] {
                    cozo::DataValue::Str(s) => s.to_string(),
                    _ => String::new(),
                },
            )
        })
        .collect();

    assert!(
        pairs.iter().any(|(a, b)| a == "alpha" && b == "beta"),
        "expected intra-file edge alpha -> beta in call_edge, got {pairs:?}"
    );

    assert!(
        pairs.iter().any(|(a, b)| a == "gamma" && b == "beta"),
        "expected cross-file edge gamma -> beta in call_edge, got {pairs:?}"
    );

    // *call_site is unchanged: every call expression still emits a row.
    let call_sites = store
        .run_query("?[count(id)] := *call_site{id}", BTreeMap::new())
        .expect("call_site count");
    let n = match &call_sites.rows[0][0] {
        cozo::DataValue::Num(cozo::Num::Int(i)) => *i,
        other => panic!("expected int, got {other:?}"),
    };
    assert!(n >= 2, "expected at least 2 call_site rows, got {n}");
}

#[test]
fn baseline_and_optimised_queries_return_identical_rows() {
    let dir = tempdir().expect("tempdir");

    // Two test files (so file_classification.is_test = true) and one
    // non-test file. Each contains a call. Uses `self::a::beta` style
    // import for the same reason as the prior test in this file.
    std::fs::write(
        dir.path().join("helper.rs"),
        "pub fn target() {}\n",
    )
    .expect("write helper");
    std::fs::write(
        dir.path().join("widget_test.rs"),
        "use self::helper::target;\nfn test_widget() { target(); }\n",
    )
    .expect("write widget_test");
    std::fs::write(
        dir.path().join("plain.rs"),
        "use self::helper::target;\nfn unused() { target(); }\n",
    )
    .expect("write plain");

    let workspace =
        Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
    let store = CozoStore::open_in_memory().expect("open store");
    let graph = GraphBuilder::new(&workspace, &[Language::Rust])
        .build(&store)
        .expect("build graph");
    populate(&store, &graph, Some(&workspace)).expect("populate");

    let baseline = std::fs::read_to_string("examples/test_to_function_map.baseline.cozoql")
        .expect("read baseline");
    let optimised = std::fs::read_to_string("examples/test_to_function_map.optimised.cozoql")
        .expect("read optimised");

    let b_rows = store.run_query(&baseline, BTreeMap::new()).expect("baseline");
    let o_rows = store.run_query(&optimised, BTreeMap::new()).expect("optimised");

    let mut b: Vec<Vec<String>> = b_rows
        .rows
        .iter()
        .map(|r| r.iter().map(|c| format!("{c:?}")).collect())
        .collect();
    let mut o: Vec<Vec<String>> = o_rows
        .rows
        .iter()
        .map(|r| r.iter().map(|c| format!("{c:?}")).collect())
        .collect();
    b.sort();
    o.sort();

    assert_eq!(
        b, o,
        "baseline and optimised queries returned different row sets"
    );
    // Sanity: we should have rows for the test file only, not for plain.rs.
    assert!(!b.is_empty(), "expected at least one row (test_widget -> target)");
    for row in &o {
        assert!(
            row[0].contains("widget_test.rs"),
            "expected only test-file rows, found row from {:?}",
            row[0]
        );
    }
}
