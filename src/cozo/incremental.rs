//! Incremental refresh.
//!
//! Phase 1 of the Datalog-model migration. The old incremental path
//! relied on `callsite` facts to re-resolve `edge_calls` after touched
//! files were re-parsed. The new schema folds call-site info into `calls`
//! rows directly and drops the `callsite` relation, which means the
//! cozo-side re-resolver can't run from facts alone any more.
//!
//! For Phase 1, `incremental_refresh` falls back to full wipe + repopulate
//! whenever any file has changed. This is a correctness-preserving
//! regression on warm-start performance; tightening it lands alongside
//! the per-language references work in Phase 6 (issue #16), where the
//! per-call-site facts that drive resolution come back as `references`
//! rows (`ref_kind = "type_use"` or call-site equivalents).
//!
//! `workspace_diff` is kept as-is since callers (`main.rs`, `server.rs`)
//! use it to decide whether to skip rebuild entirely on a clean warm
//! start.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use cozo::DataValue;

use crate::graph::CodeGraph;
use crate::graph::builder::GraphBuilder;
use crate::language::Language;
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

    pub fn touched(&self) -> impl Iterator<Item = &str> {
        self.added
            .iter()
            .chain(self.modified.iter())
            .map(|s| s.as_str())
    }
}

/// Compute the per-file diff between `build_meta_files` and the current
/// workspace state.
pub fn workspace_diff(store: &CozoStore, workspace: &Workspace) -> Result<WorkspaceDiff> {
    let rows = store
        .run_query(
            "?[path, size, mtime] := \
             *build_meta_files{file_path: path, size, mtime}",
            std::collections::BTreeMap::new(),
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
    let current_paths: HashSet<&str> = workspace.files().iter().map(|s| s.as_str()).collect();

    for path in stored.keys() {
        if !current_paths.contains(path.as_str()) {
            diff.removed.push(path.clone());
        }
    }

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

/// Phase 1 stub: in the old schema this scrubbed per-file facts across
/// every relation that referenced `file_path` or symbol ids derived from
/// it. The new schema's expanded relation set makes per-file deletion
/// without a foreign-key story expensive and error-prone; the safer
/// Phase 1 approach is full wipe + repopulate via [`incremental_refresh`].
///
/// Kept exported so the public API surface doesn't change; calling it
/// directly is a no-op and the caller should use `incremental_refresh`.
pub fn delete_file_facts(_store: &CozoStore, _file_path: &str) -> Result<()> {
    Ok(())
}

/// Phase 1 stub: cross-file edge re-resolution previously rebuilt
/// `edge_calls` + `edge_imports` from `callsite` and `raw_import` facts.
/// The new schema drops `callsite` (call sites fold into `calls` rows),
/// so this becomes a no-op. The graph builder resolves cross-file edges
/// at build time; `incremental_refresh` calls `populate` on a fresh
/// `CodeGraph`, which captures them.
pub fn resolve_cross_file_edges(_store: &CozoStore) -> Result<()> {
    Ok(())
}

/// Apply an incremental refresh.
///
/// Phase 1: any non-empty diff triggers a full wipe + repopulate from a
/// fresh `CodeGraph` over the entire workspace. True incremental refresh
/// (touching only changed files) returns once the per-language references
/// extraction in Phase 6 (issue #16) provides the per-call-site facts the
/// resolver needs.
pub fn incremental_refresh(
    store: &CozoStore,
    workspace: &Workspace,
    languages: &[Language],
    diff: &WorkspaceDiff,
) -> Result<()> {
    if diff.is_empty() {
        return Ok(());
    }
    tracing::info!(
        added = diff.added.len(),
        modified = diff.modified.len(),
        removed = diff.removed.len(),
        "incremental refresh: wiping + rebuilding"
    );
    super::wipe_workspace_relations(store)?;
    let graph = GraphBuilder::new(workspace, languages).build(store)?;
    super::populate(store, &graph, Some(workspace))?;
    Ok(())
}

/// Build a CodeGraph covering only the named subset of files. Retained
/// for potential reuse once per-file refresh comes back.
#[allow(dead_code)]
fn build_partial_graph(
    store: &CozoStore,
    workspace: &Workspace,
    languages: &[Language],
    files: &[String],
) -> Result<CodeGraph> {
    let allowed: HashSet<&str> = files.iter().map(|s| s.as_str()).collect();
    let partial = workspace.subset(|path| allowed.contains(path));
    GraphBuilder::new(&partial, languages).build(store)
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
        let store = CozoStore::open_in_memory().expect("store");
        let graph = GraphBuilder::new(&ws, &[Language::Rust])
            .build(&store)
            .expect("build");
        super::super::populate(&store, &graph, Some(&ws)).expect("populate");

        let d = workspace_diff(&store, &ws).expect("diff");
        assert!(d.is_empty(), "expected empty diff, got {:?}", d);

        std::fs::write(dir.path().join("c.rs"), "fn c() {}\n").expect("c");
        std::thread::sleep(std::time::Duration::from_secs(1));
        std::fs::write(dir.path().join("b.rs"), "fn b2() {}\n").expect("mod b");
        std::fs::remove_file(dir.path().join("a.rs")).expect("rm a");

        let ws2 = Workspace::load(dir.path(), &[Language::Rust], None).expect("reload");
        let d = workspace_diff(&store, &ws2).expect("diff");
        assert!(d.added.contains(&"c.rs".to_string()));
        assert!(d.modified.contains(&"b.rs".to_string()));
        assert!(d.removed.contains(&"a.rs".to_string()));
    }

    #[test]
    fn incremental_round_trip_full_rebuild_phase1() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("a.rs"),
            "use self::b::beta;\nfn alpha() { beta(); }\n",
        )
        .expect("a");
        std::fs::write(dir.path().join("b.rs"), "pub fn beta() {}\n").expect("b");

        let ws = Workspace::load(dir.path(), &[Language::Rust], None).expect("load");
        let store = CozoStore::open_in_memory().expect("store");
        let graph = GraphBuilder::new(&ws, &[Language::Rust])
            .build(&store)
            .expect("build");
        super::super::populate(&store, &graph, Some(&ws)).expect("populate");

        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *calls{caller_id, callee_id}, \
                 *symbol{id: caller_id, name: caller}, \
                 *symbol{id: callee_id, name: callee}",
                std::collections::BTreeMap::new(),
            )
            .expect("query");
        assert!(
            calls
                .rows
                .iter()
                .any(|r| r[0] == DataValue::from("alpha") && r[1] == DataValue::from("beta")),
            "baseline alpha->beta missing"
        );

        std::fs::remove_file(dir.path().join("a.rs")).expect("rm a");
        std::fs::write(
            dir.path().join("c.rs"),
            "use self::b::beta;\nfn gamma() { beta(); }\n",
        )
        .expect("c");

        let ws2 = Workspace::load(dir.path(), &[Language::Rust], None).expect("reload");
        let d = workspace_diff(&store, &ws2).expect("diff");
        assert!(d.removed.contains(&"a.rs".to_string()));
        assert!(d.added.contains(&"c.rs".to_string()));

        incremental_refresh(&store, &ws2, &[Language::Rust], &d).expect("incremental");

        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *calls{caller_id, callee_id}, \
                 *symbol{id: caller_id, name: caller}, \
                 *symbol{id: callee_id, name: callee}",
                std::collections::BTreeMap::new(),
            )
            .expect("query");
        assert!(
            !calls.rows.iter().any(|r| r[0] == DataValue::from("alpha")),
            "stale alpha row still present: {:?}",
            calls.rows
        );
        assert!(
            calls
                .rows
                .iter()
                .any(|r| r[0] == DataValue::from("gamma") && r[1] == DataValue::from("beta")),
            "expected gamma->beta after incremental, got {:?}",
            calls.rows
        );
    }
}
