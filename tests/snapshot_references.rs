//! Per-language references snapshot tests (Issue #16).

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

fn snapshot_refs_for(
    lang_name: &str,
    languages: &[Language],
    bench_rel: &str,
    counts: &[(&str, &str)],
) {
    let Some(bench) = benchmark_dir(bench_rel) else {
        eprintln!("skipping {lang_name}: benchmark {bench_rel} not found");
        return;
    };
    let ws = Workspace::load(&bench, languages, None).expect("load workspace");
    let graph = GraphBuilder::new(&ws, languages)
        .build()
        .expect("build graph");
    let store = CozoStore::open_in_memory().expect("open store");
    populate(&store, &graph, Some(&ws)).expect("populate");

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
        .join(format!("tests/snapshots/{lang_name}/references.expected"));
    let expected = read_expected(&expected_path);

    let mut mismatches: Vec<(String, i64, i64)> = Vec::new();
    for (key, want) in &expected {
        let got = actual.get(key).copied().unwrap_or(0);
        if got != *want {
            mismatches.push((key.clone(), *want, got));
        }
    }
    if !mismatches.is_empty() {
        let mut dump = String::new();
        for (k, _, got) in &mismatches {
            dump.push_str(&format!("{k} = {got}\n"));
        }
        panic!(
            "[{lang_name}] {} mismatches against tests/snapshots/{lang_name}/references.expected.\n\
             Actuals (paste in if intentional):\n{dump}",
            mismatches.len()
        );
    }
}

#[test]
fn rust_references() {
    snapshot_refs_for(
        "rust",
        &[Language::Rust],
        "rust/systems-cli",
        &[
            (
                "occurrence_total",
                "?[count(id)] := *occurrence{id}",
            ),
            (
                "scope_total",
                "?[count(id)] := *scope{id}",
            ),
            (
                "binding_total",
                "?[count(s)] := *binding{scope_id: s}",
            ),
            (
                "references_total",
                "?[count(r)] := *references{referrer_id: r}",
            ),
            (
                "references_resolved",
                "?[count(r)] := *references{referrer_id: r, referent_id: rid}, rid != null",
            ),
            (
                "references_unresolved",
                "?[count(r)] := *references{referrer_id: r, referent_id: rid}, rid == null",
            ),
        ],
    );
}
