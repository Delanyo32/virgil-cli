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

use super::{CozoStore, CozoWriter};

/// Walk every node and edge of `graph` and emit the corresponding Cozo
/// rows, flushing at the end. Uses `NodeIndex::index()` as the monotonic
/// integer id for `symbol`/`callsite` rows.
pub fn populate(store: &CozoStore, graph: &CodeGraph) -> Result<()> {
    let mut writer = CozoWriter::new();

    for node_idx in graph.graph.node_indices() {
        let id = node_idx.index() as i64;
        match &graph.graph[node_idx] {
            NodeWeight::File { path, language } => {
                let path = graph.symbols.resolve(*path);
                writer.push_file(path, language.as_str());
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
        populate(&store, &graph).expect("populate");

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
}
