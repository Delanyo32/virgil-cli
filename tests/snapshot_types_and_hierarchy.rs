//! Per-language types + signatures + inheritance snapshot tests
//! (Issue #13).
//!
//! Skips vacuously when the benchmark sibling repo isn't checked out,
//! mirroring snapshot_baselines.rs / snapshot_locals.rs /
//! snapshot_symbol_metadata.rs.

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
    let graph = GraphBuilder::new(&ws, languages)
        .build()
        .expect("build graph");
    let store = CozoStore::open_in_memory().expect("open store");
    populate(&store, &graph, Some(&ws)).expect("populate");

    let counts: &[(&str, &str)] = &[
        ("type_total", "?[count(t)] := *type{id: t}"),
        (
            "type_primitive",
            "?[count(t)] := *type{id: t, kind: \"primitive\"}",
        ),
        ("type_named", "?[count(t)] := *type{id: t, kind: \"named\"}"),
        (
            "type_generic",
            "?[count(t)] := *type{id: t, kind: \"generic\"}",
        ),
        ("type_tuple", "?[count(t)] := *type{id: t, kind: \"tuple\"}"),
        ("type_array", "?[count(t)] := *type{id: t, kind: \"array\"}"),
        (
            "type_function",
            "?[count(t)] := *type{id: t, kind: \"function\"}",
        ),
        (
            "type_intersection",
            "?[count(t)] := *type{id: t, kind: \"intersection\"}",
        ),
        (
            "type_resolved_canonical",
            "?[count(t)] := *type{id: t, canonical_name: c}, c != null",
        ),
        ("parameter_total", "?[count(p)] := *parameter{id: p}"),
        (
            "parameter_with_type",
            "?[count(p)] := *parameter{id: p, type_id: tid}, tid != null",
        ),
        (
            "returns_total",
            "?[count(f)] := *returns_type{function_id: f}",
        ),
        ("extends_total", "?[count(c)] := *extends{child_id: c}"),
        ("implements_total", "?[count(i)] := *implements{impl_id: i}"),
        (
            "field_type_total",
            "?[count(s)] := *field_type{symbol_id: s}",
        ),
        ("throws_total", "?[count(f)] := *throws{function_id: f}"),
        (
            "throws_joinable_with_symbol",
            "?[count(f)] := *throws{function_id: f}, *symbol{id: f}",
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
        "tests/snapshots/{lang_name}/types-and-hierarchy.expected"
    ));
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
            "[{lang_name}] {} mismatches against tests/snapshots/{lang_name}/types-and-hierarchy.expected.\n\
             Actuals (paste in if intentional):\n{dump}",
            mismatches.len()
        );
    }
}

#[test]
fn rust_types_and_hierarchy() {
    snapshot_for("rust", &[Language::Rust], "rust/systems-cli");
}

#[test]
fn typescript_types_and_hierarchy() {
    snapshot_for(
        "typescript",
        &[Language::TypeScript, Language::Tsx],
        "typescript/nextjs-dashboard",
    );
}

#[test]
fn python_types_and_hierarchy() {
    snapshot_for("python", &[Language::Python], "python/technical-debt");
}

#[test]
fn go_types_and_hierarchy() {
    snapshot_for("go", &[Language::Go], "go/http-service");
}

#[test]
fn java_types_and_hierarchy() {
    snapshot_for("java", &[Language::Java], "java/spring-api");
}

#[test]
fn php_types_and_hierarchy() {
    snapshot_for("php", &[Language::Php], "php/laravel-store");
}

#[test]
fn c_types_and_hierarchy() {
    snapshot_for("c", &[Language::C], "c/embedded-sensors");
}

#[test]
fn cpp_types_and_hierarchy() {
    snapshot_for("cpp", &[Language::Cpp], "cpp/data-processor");
}

#[test]
fn csharp_types_and_hierarchy() {
    snapshot_for("csharp", &[Language::CSharp], "csharp/dotnet-api");
}
