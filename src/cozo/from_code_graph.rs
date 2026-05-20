//! Populate a [`CozoStore`] from a finished [`CodeGraph`].
//!
//! For issue 02 this is the wiring between the legacy in-memory graph and
//! the new fact store: both end up populated additively, so users can
//! query either one. Later phases (06) delete the legacy graph and the
//! absorber writes Cozo rows directly, at which point this module goes
//! away.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;

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
    populate_with_id_offset(store, graph, workspace, 0)
}

/// Same as [`populate`] but offsets every `symbol`/`callsite` id by
/// `id_offset`. Used by the incremental refresh path so newly-parsed
/// nodes get ids beyond the existing store's max, avoiding collisions.
pub fn populate_with_id_offset(
    store: &CozoStore,
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    id_offset: i64,
) -> Result<()> {
    let mut writer = CozoWriter::new();

    for node_idx in graph.node_indices() {
        let id = id_offset + node_idx as i64;
        match &graph.nodes[node_idx] {
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
                    caller_symbol.map(|idx| id_offset + idx as i64),
                    test_name,
                );
            }
        }
    }

    for source_idx in graph.node_indices() {
        for (target_idx, weight) in &graph.out_edges[source_idx] {
            let from = id_offset + source_idx as i64;
            let to = id_offset + *target_idx as i64;
            match weight {
                EdgeWeight::DefinedIn => {
                    if let NodeWeight::File { path, .. } = &graph.nodes[*target_idx] {
                        let path = graph.symbols.resolve(*path);
                        writer.push_edge_defined_in(from, path);
                    }
                }
                EdgeWeight::Calls => {
                    writer.push_edge_calls(from, to);
                }
                EdgeWeight::Imports => {
                    if let (NodeWeight::File { path: from_p, .. }, NodeWeight::File { path: to_p, .. }) =
                        (&graph.nodes[source_idx], &graph.nodes[*target_idx])
                    {
                        let from_path = graph.symbols.resolve(*from_p);
                        let to_path = graph.symbols.resolve(*to_p);
                        writer.push_edge_imports(from_path, to_path);
                    }
                }
                EdgeWeight::Exports => {
                    if let NodeWeight::File { path, .. } = &graph.nodes[source_idx] {
                        let file_path = graph.symbols.resolve(*path);
                        writer.push_edge_exports(file_path, to);
                    }
                }
                EdgeWeight::Contains => {
                    writer.push_edge_contains(from, to);
                }
            }
        }
    }

    // Persist raw imports per file so incremental refresh can re-resolve
    // edge_imports without re-parsing every unchanged file. The language
    // column is filled in from the workspace's `file_language` when we
    // resolve, so we leave it empty here.
    for (file_path, imports) in &graph.raw_imports {
        let lang_str = workspace
            .and_then(|ws| ws.file_language(file_path))
            .map(|l| l.as_str())
            .unwrap_or("");
        for (idx, import) in imports.iter().enumerate() {
            writer.push_raw_import(
                file_path,
                idx as i64,
                &import.module_specifier,
                lang_str,
                &import.kind,
            );
        }
    }

    if let Some(ws) = workspace {
        record_build_meta_files(ws, &mut writer);
    }

    writer.flush(store)
}

/// Wipe every relation populated by [`populate`] (plus build_meta_files
/// and file_classification + nolint). Used before a rebuild so stale rows
/// don't linger when the workspace has changed.
pub fn wipe_workspace_relations(store: &CozoStore) -> Result<()> {
    for stmt in [
        "?[path] := *file{path} :rm file {path}",
        "?[id] := *symbol{id} :rm symbol {id}",
        "?[id] := *callsite{id} :rm callsite {id}",
        "?[symbol_id, file_path] := *edge_defined_in{symbol_id, file_path} \
         :rm edge_defined_in {symbol_id, file_path}",
        "?[caller_id, callee_id] := *edge_calls{caller_id, callee_id} \
         :rm edge_calls {caller_id, callee_id}",
        "?[from_path, to_path] := *edge_imports{from_path, to_path} \
         :rm edge_imports {from_path, to_path}",
        "?[file_path, symbol_id] := *edge_exports{file_path, symbol_id} \
         :rm edge_exports {file_path, symbol_id}",
        "?[parent_id, child_id] := *edge_contains{parent_id, child_id} \
         :rm edge_contains {parent_id, child_id}",
        "?[path] := *file_classification{path} :rm file_classification {path}",
        "?[file_path, line] := *nolint{file_path, line} :rm nolint {file_path, line}",
        "?[file_path] := *build_meta_files{file_path} :rm build_meta_files {file_path}",
    ] {
        store
            .run_script(stmt, BTreeMap::new())
            .map_err(|e| anyhow::anyhow!("wipe failed for `{stmt}`: {e}"))?;
    }
    Ok(())
}

/// Returns `true` when the stored `build_meta_files` rows match the
/// workspace's current files (same set + same size + same mtime). When
/// the workspace has no on-disk root (S3), only the file set is
/// compared — sizes/mtimes aren't available there.
pub fn is_warm_compatible(store: &CozoStore, workspace: &Workspace) -> Result<bool> {
    let rows = store
        .run_query(
            "?[path, hash, size, mtime] := \
             *build_meta_files{file_path: path, hash, size, mtime}",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow::anyhow!("warm-check query failed: {e}"))?;

    let stored: std::collections::HashMap<String, (i64, i64)> = rows
        .rows
        .into_iter()
        .filter_map(|r| {
            let path = match &r[0] {
                DataValue::Str(s) => s.to_string(),
                _ => return None,
            };
            let size = match &r[2] {
                DataValue::Num(cozo::Num::Int(i)) => *i,
                _ => return None,
            };
            let mtime = match &r[3] {
                DataValue::Num(cozo::Num::Int(i)) => *i,
                _ => return None,
            };
            Some((path, (size, mtime)))
        })
        .collect();

    let current = workspace.files();
    if current.len() != stored.len() {
        return Ok(false);
    }

    let root = workspace.root();
    let on_disk = root.exists();
    for path in current {
        let Some(&(prev_size, prev_mtime)) = stored.get(path) else {
            return Ok(false);
        };
        if !on_disk {
            // S3 workspace — no mtime available. Trust the file set check
            // we already did (count + path-by-path).
            continue;
        }
        let full = root.join(path);
        let meta = match std::fs::metadata(&full) {
            Ok(m) => m,
            Err(_) => return Ok(false),
        };
        let size_now = meta.len() as i64;
        let mtime_now = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if size_now != prev_size || mtime_now != prev_mtime {
            return Ok(false);
        }
    }
    Ok(true)
}

fn record_build_meta_files(workspace: &Workspace, writer: &mut CozoWriter) {
    let root = workspace.root();
    let on_disk = root.exists();
    for path in workspace.files() {
        let (size, mtime) = if on_disk {
            let full = root.join(path);
            let meta = std::fs::metadata(&full).ok();
            (
                meta.as_ref().map(|m| m.len() as i64).unwrap_or(0),
                meta.and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            )
        } else {
            (0, 0)
        };
        writer.push_build_meta_file(path, "", size, mtime);
    }
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
