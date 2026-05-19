//! Populate a [`CozoStore`] from a finished [`CodeGraph`].
//!
//! For issue 02 this is the wiring between the legacy in-memory graph and
//! the new fact store: both end up populated additively, so users can
//! query either one. Later phases (06) delete the legacy graph and the
//! absorber writes Cozo rows directly, at which point this module goes
//! away.

use anyhow::Result;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};
use crate::classify::{is_barrel_file, is_test_file};
use crate::storage::workspace::Workspace;

use super::{CozoStore, CozoWriter};

/// Walk every node and edge of `graph` and emit the corresponding Cozo
/// rows, flushing at the end. Uses `NodeIndex::index()` as the monotonic
/// integer id for `symbol`/`callsite` rows.
///
/// When `workspace` is provided, also populates `file_classification` and
/// scans each file's source for `nolint` comments. Without a workspace
/// those derived relations stay empty (relation still exists in the
/// schema).
pub fn populate(
    store: &CozoStore,
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
) -> Result<()> {
    let mut writer = CozoWriter::new();

    for node_idx in graph.graph.node_indices() {
        let id = node_idx.index() as i64;
        match &graph.graph[node_idx] {
            NodeWeight::File { path, language } => {
                let path = graph.symbols.resolve(*path);
                writer.push_file(path, language.as_str());
                if let Some(ws) = workspace {
                    let is_generated = ws
                        .read_file(path)
                        .map(|src| is_generated_marker(&src))
                        .unwrap_or(false);
                    writer.push_file_classification(
                        path,
                        is_test_file(path),
                        is_barrel_file(path),
                        is_generated,
                    );
                    if let Some(src) = ws.read_file(path) {
                        extract_nolints(path, &src, &mut writer);
                    }
                } else {
                    writer.push_file_classification(
                        path,
                        is_test_file(path),
                        is_barrel_file(path),
                        false,
                    );
                }
            }
            NodeWeight::Symbol {
                name,
                kind,
                file_path,
                start_line,
                end_line,
                exported,
            } => {
                let name = graph.symbols.resolve(*name);
                let file_path = graph.symbols.resolve(*file_path);
                let kind_str = kind.to_string();
                writer.push_symbol(
                    id,
                    name,
                    &kind_str,
                    file_path,
                    *start_line as i64,
                    *end_line as i64,
                    *exported,
                );
            }
            NodeWeight::CallSite {
                name,
                file_path,
                line,
                enclosing_test_name,
                caller_symbol,
                ..
            } => {
                let name = graph.symbols.resolve(*name);
                let file_path = graph.symbols.resolve(*file_path);
                let test_name = enclosing_test_name.map(|s| graph.symbols.resolve(s));
                writer.push_callsite(
                    id,
                    name,
                    file_path,
                    *line as i64,
                    caller_symbol.map(|idx| idx.index() as i64),
                    test_name,
                );
            }
            // Parameter / ExternalSource / CfgExit aren't part of the issue 02
            // cross-function schema — they land with CFG facts in issue 03.
            _ => {}
        }
    }

    for edge_ref in graph.graph.edge_references() {
        let from = edge_ref.source().index() as i64;
        let to = edge_ref.target().index() as i64;
        match edge_ref.weight() {
            EdgeWeight::DefinedIn => {
                // edge source is a Symbol, target is a File. Translate to the
                // schema's (symbol_id, file_path) shape.
                if let NodeWeight::File { path, .. } = &graph.graph[edge_ref.target()] {
                    let path = graph.symbols.resolve(*path);
                    writer.push_edge_defined_in(from, path);
                }
            }
            EdgeWeight::Calls => {
                writer.push_edge_calls(from, to);
            }
            EdgeWeight::Imports => {
                // File -> File. Schema wants (from_path, to_path).
                if let (NodeWeight::File { path: from_p, .. }, NodeWeight::File { path: to_p, .. }) =
                    (&graph.graph[edge_ref.source()], &graph.graph[edge_ref.target()])
                {
                    let from_path = graph.symbols.resolve(*from_p);
                    let to_path = graph.symbols.resolve(*to_p);
                    writer.push_edge_imports(from_path, to_path);
                }
            }
            EdgeWeight::Exports => {
                // File -> Symbol. Schema wants (file_path, symbol_id).
                if let NodeWeight::File { path, .. } = &graph.graph[edge_ref.source()] {
                    let file_path = graph.symbols.resolve(*path);
                    writer.push_edge_exports(file_path, to);
                }
            }
            EdgeWeight::Contains => {
                writer.push_edge_contains(from, to);
            }
            // FlowsTo / SanitizedBy / Acquires / ReleasedBy / ExitsVia all
            // belong to CFG / taint / resource passes and land in later issues.
            _ => {}
        }
    }

    writer.flush(store)
}

/// Heuristic: a file is "generated" if its first 20 lines contain a
/// well-known marker. Matches the conventions of the major generators
/// (rustfmt-skip @generated, protoc, prost, codegen, etc.) without
/// claiming to be exhaustive.
fn is_generated_marker(source: &str) -> bool {
    const MARKERS: &[&str] = &[
        "@generated",
        "Code generated by",
        "DO NOT EDIT",
        "GENERATED FILE",
        "automatically generated",
    ];
    for (i, line) in source.lines().enumerate() {
        if i >= 20 {
            break;
        }
        for m in MARKERS {
            if line.contains(m) {
                return true;
            }
        }
    }
    false
}

/// Scan `source` for `nolint:<pattern>` directives and emit rows.
/// Supports the two prefix styles in use across the supported languages:
///
/// - `// nolint:<pattern>` (C-family, Rust, JS/TS, Java, C#, Go, PHP)
/// - `# nolint:<pattern>` (Python)
fn extract_nolints(file_path: &str, source: &str, writer: &mut CozoWriter) {
    for (i, line) in source.lines().enumerate() {
        let line_no = (i + 1) as i64;
        if let Some(pat) = find_nolint(line) {
            writer.push_nolint(file_path, line_no, pat);
        }
    }
}

fn find_nolint(line: &str) -> Option<&str> {
    const PREFIXES: &[&str] = &["// nolint:", "# nolint:", "/* nolint:"];
    for prefix in PREFIXES {
        if let Some(start) = line.find(prefix) {
            let rest = &line[start + prefix.len()..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '*' || c == ',')
                .unwrap_or(rest.len());
            let pat = rest[..end].trim();
            if !pat.is_empty() {
                return Some(pat);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use crate::graph::builder::GraphBuilder;
    use crate::language::Language;
    use crate::storage::workspace::Workspace;

    use super::*;

    #[test]
    fn populate_writes_symbols_and_call_edges_for_a_tiny_rust_workspace() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("lib.rs"),
            "fn alpha() { beta(); }\nfn beta() {}\n",
        )
        .expect("write");

        let workspace =
            Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build()
            .expect("build graph");
        let store = CozoStore::open_in_memory().expect("open store");
        populate(&store, &graph, Some(&workspace)).expect("populate");

        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *edge_calls{caller_id, callee_id}, \
                 *symbol{id: caller_id, name: caller}, \
                 *symbol{id: callee_id, name: callee}",
                BTreeMap::new(),
            )
            .expect("query");
        let found_alpha_calls_beta = calls.rows.iter().any(|r| {
            r[0] == cozo::DataValue::from("alpha") && r[1] == cozo::DataValue::from("beta")
        });
        assert!(
            found_alpha_calls_beta,
            "expected alpha -> beta call edge, got rows: {:?}",
            calls.rows
        );

        let files = store
            .run_query("?[p] := *file{path: p}", BTreeMap::new())
            .expect("file query");
        assert_eq!(files.rows.len(), 1);
        assert_eq!(files.rows[0][0], cozo::DataValue::from("lib.rs"));
    }

    #[test]
    fn populate_classifies_test_barrel_and_generated_files() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mod.rs"), "pub mod inner;\n").expect("write mod");
        std::fs::create_dir(dir.path().join("inner")).expect("dir");
        std::fs::write(
            dir.path().join("inner").join("mod.rs"),
            "pub fn ok() {}\n",
        )
        .expect("write inner mod");
        std::fs::create_dir(dir.path().join("tests")).expect("test dir");
        std::fs::write(
            dir.path().join("tests").join("user_flow_test.rs"),
            "fn test_thing() {}\n",
        )
        .expect("write test");
        std::fs::write(
            dir.path().join("inner").join("generated.rs"),
            "// @generated by prost\npub struct X;\n",
        )
        .expect("write gen");

        let workspace =
            Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build()
            .expect("build graph");
        let store = CozoStore::open_in_memory().expect("open store");
        populate(&store, &graph, Some(&workspace)).expect("populate");

        let rows = store
            .run_query(
                "?[p, t, b, g] := *file_classification{path: p, is_test: t, \
                 is_barrel: b, is_generated: g}",
                BTreeMap::new(),
            )
            .expect("query");
        let by_path: std::collections::HashMap<String, (bool, bool, bool)> = rows
            .rows
            .iter()
            .map(|r| {
                let p = match &r[0] {
                    cozo::DataValue::Str(s) => s.to_string(),
                    other => panic!("path str expected, got {other:?}"),
                };
                let t = matches!(r[1], cozo::DataValue::Bool(true));
                let b = matches!(r[2], cozo::DataValue::Bool(true));
                let g = matches!(r[3], cozo::DataValue::Bool(true));
                (p, (t, b, g))
            })
            .collect();
        assert_eq!(
            by_path.get("tests/user_flow_test.rs"),
            Some(&(true, false, false))
        );
        assert_eq!(by_path.get("mod.rs"), Some(&(false, true, false)));
        assert_eq!(
            by_path.get("inner/generated.rs"),
            Some(&(false, false, true))
        );
    }

    #[test]
    fn populate_extracts_nolint_comments() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("lib.rs"),
            "// nolint:unused_var\nfn x() {}\n// nolint:dead_code blah\nfn y() {}\n",
        )
        .expect("write");

        let workspace =
            Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build()
            .expect("build graph");
        let store = CozoStore::open_in_memory().expect("open store");
        populate(&store, &graph, Some(&workspace)).expect("populate");

        let rows = store
            .run_query(
                "?[line, pattern] := *nolint{file_path: 'lib.rs', line, suppressed_pattern: pattern}",
                BTreeMap::new(),
            )
            .expect("query");
        let entries: Vec<(i64, String)> = rows
            .rows
            .iter()
            .map(|r| {
                let line = match &r[0] {
                    cozo::DataValue::Num(n) => match n {
                        cozo::Num::Int(i) => *i,
                        other => panic!("int expected, got {other:?}"),
                    },
                    other => panic!("num expected, got {other:?}"),
                };
                let pat = match &r[1] {
                    cozo::DataValue::Str(s) => s.to_string(),
                    other => panic!("str expected, got {other:?}"),
                };
                (line, pat)
            })
            .collect();
        assert!(
            entries.contains(&(1, "unused_var".to_string())),
            "missing line-1 nolint, got {entries:?}"
        );
        assert!(
            entries.contains(&(3, "dead_code".to_string())),
            "missing line-3 nolint, got {entries:?}"
        );
    }
}
