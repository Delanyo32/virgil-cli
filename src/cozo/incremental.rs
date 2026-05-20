//! Incremental refresh (issue 08).
//!
//! Compares the current workspace against `build_meta_files`, deletes
//! facts for removed/modified files, re-parses just the changed files, and
//! re-resolves cross-file edges (edge_calls + edge_imports) against the
//! union of unchanged + new facts.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, anyhow};
use cozo::DataValue;

use crate::graph::CodeGraph;
use crate::graph::builder::GraphBuilder;
use crate::language::Language;
use crate::languages;
use crate::storage::workspace::Workspace;

use super::CozoStore;

/// Per-file changes since the last build, derived from a `build_meta_files`
/// vs. current-workspace comparison.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceDiff {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
}

impl WorkspaceDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.removed.is_empty()
    }

    /// Files that need re-parsing on this incremental pass.
    pub fn touched(&self) -> impl Iterator<Item = &str> {
        self.added.iter().chain(self.modified.iter()).map(|s| s.as_str())
    }
}

/// Compute the per-file diff between `build_meta_files` and the current
/// workspace state.
pub fn workspace_diff(store: &CozoStore, workspace: &Workspace) -> Result<WorkspaceDiff> {
    let rows = store
        .run_query(
            "?[path, size, mtime] := \
             *build_meta_files{file_path: path, size, mtime}",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("workspace_diff query failed: {e}"))?;

    let stored: HashMap<String, (i64, i64)> = rows
        .rows
        .into_iter()
        .filter_map(|r| {
            let path = match &r[0] {
                DataValue::Str(s) => s.to_string(),
                _ => return None,
            };
            let size = match &r[1] {
                DataValue::Num(cozo::Num::Int(i)) => *i,
                _ => return None,
            };
            let mtime = match &r[2] {
                DataValue::Num(cozo::Num::Int(i)) => *i,
                _ => return None,
            };
            Some((path, (size, mtime)))
        })
        .collect();

    let mut diff = WorkspaceDiff::default();
    let root = workspace.root();
    let on_disk = root.exists();
    let current_paths: HashSet<&str> =
        workspace.files().iter().map(|s| s.as_str()).collect();

    // Removed: in stored but not in current
    for path in stored.keys() {
        if !current_paths.contains(path.as_str()) {
            diff.removed.push(path.clone());
        }
    }

    // Added + modified
    for path in workspace.files() {
        let current_meta = if on_disk {
            let full = root.join(path);
            std::fs::metadata(&full).ok().map(|m| {
                let size = m.len() as i64;
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                (size, mtime)
            })
        } else {
            Some((0, 0))
        };
        match (stored.get(path), current_meta) {
            (None, _) => diff.added.push(path.clone()),
            (Some(&(prev_size, prev_mtime)), Some((size, mtime))) => {
                if !on_disk {
                    // S3 workspace — no mtime, trust set-equality only.
                    continue;
                }
                if size != prev_size || mtime != prev_mtime {
                    diff.modified.push(path.clone());
                }
            }
            (Some(_), None) => diff.modified.push(path.clone()),
        }
    }
    Ok(diff)
}

/// Delete every fact owned by `file_path`. Cascades through:
/// `file` / `symbol` / `callsite` / `edge_defined_in` / `edge_exports` /
/// `edge_contains` / `raw_import` / `file_classification` / `nolint` /
/// `build_meta_files`. Cross-file edges (`edge_calls`, `edge_imports`)
/// are re-resolved separately via [`resolve_cross_file_edges`].
pub fn delete_file_facts(store: &CozoStore, file_path: &str) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("p".to_string(), DataValue::from(file_path));

    let symbol_ids = collect_symbol_ids_for_file(store, file_path)?;
    let callsite_ids = collect_callsite_ids_for_file(store, file_path)?;

    // 1. Wipe edge_contains rows where parent OR child is a deleted symbol/callsite.
    let mut affected_ids: HashSet<i64> = HashSet::new();
    affected_ids.extend(symbol_ids.iter().copied());
    affected_ids.extend(callsite_ids.iter().copied());
    if !affected_ids.is_empty() {
        let ids_list: Vec<DataValue> = affected_ids.iter().map(|i| DataValue::from(*i)).collect();
        let mut p = BTreeMap::new();
        p.insert("ids".to_string(), DataValue::List(ids_list.clone()));
        store
            .run_script(
                "?[parent_id, child_id] := \
                 *edge_contains{parent_id, child_id}, \
                 (parent_id in $ids or child_id in $ids) \
                 :rm edge_contains {parent_id, child_id}",
                p,
            )
            .map_err(|e| anyhow!("delete edge_contains for {file_path}: {e}"))?;

        let mut p = BTreeMap::new();
        p.insert("ids".to_string(), DataValue::List(ids_list.clone()));
        store
            .run_script(
                "?[file_path, symbol_id] := \
                 *edge_exports{file_path, symbol_id}, symbol_id in $ids \
                 :rm edge_exports {file_path, symbol_id}",
                p,
            )
            .map_err(|e| anyhow!("delete edge_exports for {file_path}: {e}"))?;
    }

    // 2. Wipe per-file facts via the file_path key.
    for stmt in [
        ("?[file_path, position] := *raw_import{file_path, position}, file_path = $p \
          :rm raw_import {file_path, position}", "raw_import"),
        ("?[symbol_id, file_path] := *edge_defined_in{symbol_id, file_path}, file_path = $p \
          :rm edge_defined_in {symbol_id, file_path}", "edge_defined_in"),
        ("?[file_path, line] := *nolint{file_path, line}, file_path = $p \
          :rm nolint {file_path, line}", "nolint"),
        ("?[path] := *file_classification{path}, path = $p \
          :rm file_classification {path}", "file_classification"),
        ("?[file_path] := *build_meta_files{file_path}, file_path = $p \
          :rm build_meta_files {file_path}", "build_meta_files"),
    ] {
        let mut p = BTreeMap::new();
        p.insert("p".to_string(), DataValue::from(file_path));
        store
            .run_script(stmt.0, p)
            .map_err(|e| anyhow!("delete {} for {file_path}: {e}", stmt.1))?;
    }

    // 3. Delete symbols + callsites in this file by id.
    if !symbol_ids.is_empty() {
        let ids: Vec<DataValue> = symbol_ids.iter().map(|i| DataValue::from(*i)).collect();
        let mut p = BTreeMap::new();
        p.insert("ids".to_string(), DataValue::List(ids));
        store
            .run_script(
                "?[id] := *symbol{id}, id in $ids :rm symbol {id}",
                p,
            )
            .map_err(|e| anyhow!("delete symbol rows for {file_path}: {e}"))?;
    }
    if !callsite_ids.is_empty() {
        let ids: Vec<DataValue> = callsite_ids.iter().map(|i| DataValue::from(*i)).collect();
        let mut p = BTreeMap::new();
        p.insert("ids".to_string(), DataValue::List(ids));
        store
            .run_script(
                "?[id] := *callsite{id}, id in $ids :rm callsite {id}",
                p,
            )
            .map_err(|e| anyhow!("delete callsite rows for {file_path}: {e}"))?;
    }

    // 4. Finally, delete the file row.
    store
        .run_script(
            "?[path] := *file{path}, path = $p :rm file {path}",
            params,
        )
        .map_err(|e| anyhow!("delete file row for {file_path}: {e}"))?;
    Ok(())
}

fn collect_symbol_ids_for_file(store: &CozoStore, file_path: &str) -> Result<Vec<i64>> {
    let mut p = BTreeMap::new();
    p.insert("p".to_string(), DataValue::from(file_path));
    let rows = store
        .run_query(
            "?[id] := *symbol{id, file_path}, file_path = $p",
            p,
        )
        .map_err(|e| anyhow!("query symbol ids for {file_path}: {e}"))?;
    Ok(rows
        .rows
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(DataValue::Num(cozo::Num::Int(i))) => Some(i),
            _ => None,
        })
        .collect())
}

fn collect_callsite_ids_for_file(store: &CozoStore, file_path: &str) -> Result<Vec<i64>> {
    let mut p = BTreeMap::new();
    p.insert("p".to_string(), DataValue::from(file_path));
    let rows = store
        .run_query(
            "?[id] := *callsite{id, file_path}, file_path = $p",
            p,
        )
        .map_err(|e| anyhow!("query callsite ids for {file_path}: {e}"))?;
    Ok(rows
        .rows
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(DataValue::Num(cozo::Num::Int(i))) => Some(i),
            _ => None,
        })
        .collect())
}

/// Re-resolve `edge_calls` and `edge_imports` against the current
/// `*symbol`, `*callsite`, `*raw_import`, and `*file` rows. Idempotent —
/// always replaces the entire edge set, no incremental edge math.
pub fn resolve_cross_file_edges(store: &CozoStore) -> Result<()> {
    // edge_calls: for each callsite with a known caller symbol, find every
    // symbol whose name matches the callsite's name (excluding self-calls).
    // Pull rows in Rust to avoid Cozo's Null-aware semantics being awkward
    // in a :replace head.
    let pairs_rows = store
        .run_query(
            "?[caller_id, callee_id] := \
             *callsite{caller_symbol_id: caller_id, name}, \
             *symbol{id: callee_id, name}, \
             caller_id != callee_id",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("collect edge_calls candidates: {e}"))?;

    let mut pair_rows: Vec<DataValue> = Vec::with_capacity(pairs_rows.rows.len());
    for r in pairs_rows.rows {
        let (caller, callee) = match (&r[0], &r[1]) {
            (DataValue::Num(cozo::Num::Int(c)), DataValue::Num(cozo::Num::Int(d))) => (*c, *d),
            _ => continue,
        };
        pair_rows.push(DataValue::List(vec![
            DataValue::from(caller),
            DataValue::from(callee),
        ]));
    }
    // Wipe + put. `:replace` requires the full schema in some Cozo
    // versions and is finicky; explicit two-step is simpler.
    store
        .run_script(
            "?[caller_id, callee_id] := *edge_calls{caller_id, callee_id} \
             :rm edge_calls {caller_id, callee_id}",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("wipe edge_calls: {e}"))?;
    if !pair_rows.is_empty() {
        let mut p = BTreeMap::new();
        p.insert("rows".to_string(), DataValue::List(pair_rows));
        store
            .run_script(
                "?[caller_id, callee_id] <- $rows \
                 :put edge_calls {caller_id, callee_id}",
                p,
            )
            .map_err(|e| anyhow!("put edge_calls: {e}"))?;
    }

    // edge_imports: pull raw imports, resolve each via the language-specific
    // resolver, then :replace.
    let raw = store
        .run_query(
            "?[from_path, raw_path, language] := \
             *raw_import{file_path: from_path, raw_path, language}",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("query raw_imports: {e}"))?;
    let known_files: HashSet<String> = {
        let rows = store
            .run_query(
                "?[p] := *file{path: p}",
                BTreeMap::new(),
            )
            .map_err(|e| anyhow!("query files: {e}"))?;
        rows.rows
            .into_iter()
            .filter_map(|r| match r.into_iter().next() {
                Some(DataValue::Str(s)) => Some(s.to_string()),
                _ => None,
            })
            .collect()
    };

    let mut resolved: Vec<(String, String)> = Vec::new();
    for row in raw.rows {
        let from_path = match &row[0] {
            DataValue::Str(s) => s.to_string(),
            _ => continue,
        };
        let raw_path = match &row[1] {
            DataValue::Str(s) => s.to_string(),
            _ => continue,
        };
        let language_str = match &row[2] {
            DataValue::Str(s) => s.to_string(),
            _ => continue,
        };
        let Some(language) = Language::from_str(&language_str) else {
            continue;
        };
        let import_info = crate::models::ImportInfo {
            source_file: from_path.clone(),
            module_specifier: raw_path,
            imported_name: String::new(),
            local_name: String::new(),
            kind: String::new(),
            is_type_only: false,
            line: 0,
            is_external: false,
        };
        if let Some(target) =
            resolve_import_to_file(&from_path, &import_info, language, &known_files)
            && target != from_path
        {
            resolved.push((from_path, target));
        }
    }
    // Dedup
    resolved.sort();
    resolved.dedup();

    // Wipe + put. Same reasoning as edge_calls above.
    store
        .run_script(
            "?[from_path, to_path] := *edge_imports{from_path, to_path} \
             :rm edge_imports {from_path, to_path}",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("wipe edge_imports: {e}"))?;
    if !resolved.is_empty() {
        let rows_list: Vec<DataValue> = resolved
            .into_iter()
            .map(|(a, b)| DataValue::List(vec![DataValue::from(a), DataValue::from(b)]))
            .collect();
        let mut p = BTreeMap::new();
        p.insert("rows".to_string(), DataValue::List(rows_list));
        store
            .run_script(
                "?[from_path, to_path] <- $rows :put edge_imports {from_path, to_path}",
                p,
            )
            .map_err(|e| anyhow!("put edge_imports: {e}"))?;
    }

    Ok(())
}

fn resolve_import_to_file(
    source_file: &str,
    import: &crate::models::ImportInfo,
    language: Language,
    known_files: &HashSet<String>,
) -> Option<String> {
    use crate::graph::GraphNode;
    let node = languages::resolve_import(source_file, import, language, known_files)?;
    Some(match node {
        GraphNode::File(p) => p,
        GraphNode::Package(p) => p,
    })
}

/// Apply an incremental refresh: re-parse only `diff.touched()` files,
/// drop facts for `diff.removed`, then re-resolve cross-file edges.
pub fn incremental_refresh(
    store: &CozoStore,
    workspace: &Workspace,
    languages: &[Language],
    diff: &WorkspaceDiff,
) -> Result<()> {
    if diff.is_empty() {
        return Ok(());
    }

    // 1. Delete facts for removed + modified files.
    for path in &diff.removed {
        delete_file_facts(store, path)?;
    }
    for path in &diff.modified {
        delete_file_facts(store, path)?;
    }

    // 2. Re-parse + emit facts for added + modified files.
    let touched: Vec<String> = diff.touched().map(|s| s.to_string()).collect();
    if !touched.is_empty() {
        // The cheapest way to parse just N files using the existing parser
        // is to build a workspace-bounded GraphBuilder and walk only those
        // files. We don't have a per-file API, so we build a temporary
        // CodeGraph from a workspace view limited to the touched files.
        let partial_graph = build_partial_graph(workspace, languages, &touched)?;
        let id_offset = next_id_offset(store)?;
        super::from_code_graph::populate_with_id_offset(
            store,
            &partial_graph,
            Some(workspace),
            id_offset,
        )?;
    }

    // 3. Re-resolve cross-file edges over the union of old+new facts.
    resolve_cross_file_edges(store)?;
    Ok(())
}

/// Largest existing id across `*symbol` and `*callsite` + 1. Used to
/// shift NodeIndex-derived ids on incremental writes so they don't
/// collide with the existing store.
fn next_id_offset(store: &CozoStore) -> Result<i64> {
    let rows = store
        .run_query(
            "?[m] := \
             m_sym = max_or(s, -1), *symbol{id: s}, \
             m_cs = max_or(c, -1), *callsite{id: c}, \
             m = max(m_sym, m_cs)",
            BTreeMap::new(),
        )
        .or_else(|_| {
            // Fallback: query each separately when the combined form fails.
            let mut max_id: i64 = -1;
            for stmt in [
                "?[m] := *symbol{id: s}, m = max(s)",
                "?[m] := *callsite{id: c}, m = max(c)",
            ] {
                if let Ok(r) = store.run_query(stmt, BTreeMap::new()) {
                    if let Some(row) = r.rows.into_iter().next()
                        && let Some(DataValue::Num(cozo::Num::Int(i))) = row.into_iter().next()
                    {
                        max_id = max_id.max(i);
                    }
                }
            }
            // Build a synthetic NamedRows from max_id.
            let nr = cozo::NamedRows::new(
                vec!["m".to_string()],
                vec![vec![DataValue::from(max_id)]],
            );
            Ok::<_, anyhow::Error>(nr)
        })?;
    let max_existing = rows
        .rows
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next())
        .and_then(|v| match v {
            DataValue::Num(cozo::Num::Int(i)) => Some(i),
            _ => None,
        })
        .unwrap_or(-1);
    Ok(max_existing + 1)
}

/// Build a CodeGraph covering only the named subset of files.
fn build_partial_graph(
    workspace: &Workspace,
    languages: &[Language],
    files: &[String],
) -> Result<CodeGraph> {
    let allowed: HashSet<&str> = files.iter().map(|s| s.as_str()).collect();
    let partial = workspace.subset(|path| allowed.contains(path));
    GraphBuilder::new(&partial, languages).build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn diff_detects_add_modify_remove() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("a");
        std::fs::write(dir.path().join("b.rs"), "fn b() {}\n").expect("b");

        let ws = Workspace::load(dir.path(), &[Language::Rust], None).expect("load");
        let graph = GraphBuilder::new(&ws, &[Language::Rust]).build().expect("build");
        let store = CozoStore::open_in_memory().expect("store");
        super::super::populate(&store, &graph, Some(&ws)).expect("populate");

        // No changes yet — diff should be empty.
        let d = workspace_diff(&store, &ws).expect("diff");
        assert!(d.is_empty(), "expected empty diff, got {:?}", d);

        // Add a new file.
        std::fs::write(dir.path().join("c.rs"), "fn c() {}\n").expect("c");
        // Modify b.
        std::thread::sleep(std::time::Duration::from_secs(1));
        std::fs::write(dir.path().join("b.rs"), "fn b2() {}\n").expect("mod b");
        // Remove a (simulate by reloading a fresh workspace without a.rs).
        std::fs::remove_file(dir.path().join("a.rs")).expect("rm a");

        let ws2 = Workspace::load(dir.path(), &[Language::Rust], None).expect("reload");
        let d = workspace_diff(&store, &ws2).expect("diff");
        assert!(d.added.contains(&"c.rs".to_string()));
        assert!(d.modified.contains(&"b.rs".to_string()));
        assert!(d.removed.contains(&"a.rs".to_string()));
    }

    #[test]
    fn incremental_round_trip_add_modify_delete() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "fn alpha() { beta(); }\n").expect("a");
        std::fs::write(dir.path().join("b.rs"), "fn beta() {}\n").expect("b");

        let ws = Workspace::load(dir.path(), &[Language::Rust], None).expect("load");
        let graph = GraphBuilder::new(&ws, &[Language::Rust]).build().expect("build");
        let store = CozoStore::open_in_memory().expect("store");
        super::super::populate(&store, &graph, Some(&ws)).expect("populate");
        resolve_cross_file_edges(&store).expect("resolve edges");

        // baseline: alpha -> beta
        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *edge_calls{caller_id, callee_id}, \
                 *symbol{id: caller_id, name: caller}, \
                 *symbol{id: callee_id, name: callee}",
                BTreeMap::new(),
            )
            .expect("query");
        assert!(
            calls.rows.iter().any(|r| r[0] == DataValue::from("alpha")
                && r[1] == DataValue::from("beta")),
            "baseline alpha->beta missing"
        );

        // delete a.rs and add c.rs that calls beta
        std::fs::remove_file(dir.path().join("a.rs")).expect("rm a");
        std::fs::write(dir.path().join("c.rs"), "fn gamma() { beta(); }\n").expect("c");

        let ws2 = Workspace::load(dir.path(), &[Language::Rust], None).expect("reload");
        let d = workspace_diff(&store, &ws2).expect("diff");
        assert!(d.removed.contains(&"a.rs".to_string()));
        assert!(d.added.contains(&"c.rs".to_string()));

        incremental_refresh(&store, &ws2, &[Language::Rust], &d).expect("incremental");

        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *edge_calls{caller_id, callee_id}, \
                 *symbol{id: caller_id, name: caller}, \
                 *symbol{id: callee_id, name: callee}",
                BTreeMap::new(),
            )
            .expect("query");
        // alpha is gone, gamma -> beta now exists
        assert!(
            !calls.rows.iter().any(|r| r[0] == DataValue::from("alpha")),
            "stale alpha row still present: {:?}",
            calls.rows
        );
        assert!(
            calls.rows.iter().any(|r| r[0] == DataValue::from("gamma")
                && r[1] == DataValue::from("beta")),
            "expected gamma->beta after incremental, got {:?}",
            calls.rows
        );
    }
}
