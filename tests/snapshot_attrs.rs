//! Per-language `*_attrs` snapshot tests (Issue #15).
//!
//! Skips vacuously when the benchmark sibling repo isn't checked out,
//! mirroring snapshot_types_and_hierarchy.rs.

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

fn snapshot_attrs_for(
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
    let store = CozoStore::open_in_memory().expect("open store");
    let graph = GraphBuilder::new(&ws, languages)
        .build(&store)
        .expect("build graph");
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
        .join(format!("tests/snapshots/{lang_name}/attrs.expected"));
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
            "[{lang_name}] {} mismatches against tests/snapshots/{lang_name}/attrs.expected.\n\
             Actuals (paste in if intentional):\n{dump}",
            mismatches.len()
        );
    }
}

#[test]
fn rust_attrs() {
    snapshot_attrs_for(
        "rust",
        &[Language::Rust],
        "rust/systems-cli",
        &[
            (
                "rust_attrs_total",
                "?[count(s)] := *rust_attrs{symbol_id: s}",
            ),
            (
                "rust_attrs_unsafe",
                "?[count(s)] := *rust_attrs{symbol_id: s, is_unsafe: true}",
            ),
            (
                "rust_attrs_const",
                "?[count(s)] := *rust_attrs{symbol_id: s, is_const: true}",
            ),
            (
                "rust_attrs_with_derives",
                "?[count(s)] := *rust_attrs{symbol_id: s, derives: d}, length(d) > 0",
            ),
        ],
    );
}

#[test]
fn typescript_attrs() {
    snapshot_attrs_for(
        "typescript",
        &[Language::TypeScript, Language::Tsx],
        "typescript/nextjs-dashboard",
        &[
            (
                "ts_attrs_total",
                "?[count(s)] := *typescript_attrs{symbol_id: s}",
            ),
            (
                "ts_attrs_readonly",
                "?[count(s)] := *typescript_attrs{symbol_id: s, is_readonly: true}",
            ),
            (
                "ts_attrs_optional",
                "?[count(s)] := *typescript_attrs{symbol_id: s, is_optional: true}",
            ),
            (
                "ts_attrs_with_type_params",
                "?[count(s)] := *typescript_attrs{symbol_id: s, type_parameters: tp}, length(tp) > 0",
            ),
        ],
    );
}

#[test]
fn python_attrs() {
    snapshot_attrs_for(
        "python",
        &[Language::Python],
        "python/technical-debt",
        &[
            (
                "py_attrs_total",
                "?[count(s)] := *python_attrs{symbol_id: s}",
            ),
            (
                "py_attrs_generator",
                "?[count(s)] := *python_attrs{symbol_id: s, is_generator: true}",
            ),
            (
                "py_attrs_coroutine",
                "?[count(s)] := *python_attrs{symbol_id: s, is_coroutine: true}",
            ),
            (
                "py_attrs_with_decorators",
                "?[count(s)] := *python_attrs{symbol_id: s, decorators: d}, length(d) > 0",
            ),
        ],
    );
}

#[test]
fn go_attrs() {
    snapshot_attrs_for(
        "go",
        &[Language::Go],
        "go/http-service",
        &[
            ("go_attrs_total", "?[count(s)] := *go_attrs{symbol_id: s}"),
            (
                "go_attrs_exported",
                "?[count(s)] := *go_attrs{symbol_id: s, is_exported: true}",
            ),
            (
                "go_attrs_with_receiver",
                "?[count(s)] := *go_attrs{symbol_id: s, has_receiver: true}",
            ),
        ],
    );
}

#[test]
fn java_attrs() {
    snapshot_attrs_for(
        "java",
        &[Language::Java],
        "java/spring-api",
        &[
            (
                "java_attrs_total",
                "?[count(s)] := *java_attrs{symbol_id: s}",
            ),
            (
                "java_attrs_final",
                "?[count(s)] := *java_attrs{symbol_id: s, is_final: true}",
            ),
            (
                "java_attrs_with_annotations",
                "?[count(s)] := *java_attrs{symbol_id: s, annotations: a}, length(a) > 0",
            ),
            (
                "java_attrs_with_throws",
                "?[count(s)] := *java_attrs{symbol_id: s, throws_clause: t}, length(t) > 0",
            ),
        ],
    );
}

#[test]
fn php_attrs() {
    snapshot_attrs_for(
        "php",
        &[Language::Php],
        "php/laravel-store",
        &[
            ("php_attrs_total", "?[count(s)] := *php_attrs{symbol_id: s}"),
            (
                "php_attrs_final",
                "?[count(s)] := *php_attrs{symbol_id: s, is_final: true}",
            ),
            (
                "php_attrs_with_traits",
                "?[count(s)] := *php_attrs{symbol_id: s, uses_traits: t}, length(t) > 0",
            ),
        ],
    );
}

#[test]
fn c_attrs() {
    snapshot_attrs_for(
        "c",
        &[Language::C],
        "c/embedded-sensors",
        &[
            ("c_attrs_total", "?[count(s)] := *c_attrs{symbol_id: s}"),
            (
                "c_attrs_static",
                "?[count(s)] := *c_attrs{symbol_id: s, is_file_static: true}",
            ),
            (
                "c_attrs_extern",
                "?[count(s)] := *c_attrs{symbol_id: s, is_extern: true}",
            ),
            (
                "c_attrs_inline",
                "?[count(s)] := *c_attrs{symbol_id: s, is_inline: true}",
            ),
        ],
    );
}

#[test]
fn cpp_attrs() {
    snapshot_attrs_for(
        "cpp",
        &[Language::Cpp],
        "cpp/data-processor",
        &[
            ("cpp_attrs_total", "?[count(s)] := *cpp_attrs{symbol_id: s}"),
            (
                "cpp_attrs_virtual",
                "?[count(s)] := *cpp_attrs{symbol_id: s, is_virtual: true}",
            ),
            (
                "cpp_attrs_constexpr",
                "?[count(s)] := *cpp_attrs{symbol_id: s, is_constexpr: true}",
            ),
            (
                "cpp_attrs_template",
                "?[count(s)] := *cpp_attrs{symbol_id: s, is_template: true}",
            ),
        ],
    );
}

#[test]
fn csharp_attrs() {
    snapshot_attrs_for(
        "csharp",
        &[Language::CSharp],
        "csharp/dotnet-api",
        &[
            (
                "cs_attrs_total",
                "?[count(s)] := *csharp_attrs{symbol_id: s}",
            ),
            (
                "cs_attrs_sealed",
                "?[count(s)] := *csharp_attrs{symbol_id: s, is_sealed: true}",
            ),
            (
                "cs_attrs_partial",
                "?[count(s)] := *csharp_attrs{symbol_id: s, is_partial: true}",
            ),
            (
                "cs_attrs_with_attributes",
                "?[count(s)] := *csharp_attrs{symbol_id: s, attributes: a}, length(a) > 0",
            ),
        ],
    );
}
