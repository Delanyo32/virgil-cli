//! Populate a [`CozoStore`] from a finished [`CodeGraph`].
//!
//! Phase 1 of the Datalog-model migration. The existing `CodeGraph`
//! produces only a subset of the new schema's fields (line ranges, no
//! byte offsets; visibility derived from `exported`; no qualified names,
//! comments, types, references, or extends/implements). Unrendered
//! relations exist in the schema but stay empty until per-language
//! extractor enrichment lands in later phases.
//!
//! String IDs follow [ADR-0002]: `path|start_line|start_col|name|kind`.
//! `start_col` is `0` until Phase 1.5 wires byte/column offsets through
//! the graph builder.
//!
//! [ADR-0002]: docs/adr/0002-symbol-id-scheme.md

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;
use tracing::{info, info_span};

use crate::graph::CodeGraph;
use crate::models::{InheritanceKind, SymbolKind};
use crate::storage::workspace::Workspace;

use super::{CozoStore, CozoWriter};

/// After Slice B, `*file`, `*symbol`, `*span`, `*calls`, `*imports`,
/// `*raw_import`, `*scope`, `*binding`, `*occurrence`, and per-language
/// `*_attrs` rows are all streamed to Cozo during
/// `GraphBuilder::build`. This function handles only the tail work
/// that still needs cross-file symbol-id resolution against the
/// workspace symbol table: comments (`documents_id`), types/inheritance
/// (parameter / returns_type / extends / implements / throws /
/// field_type), and `build_meta_files`. Then runs the reference
/// resolver against the now-flushed fact relations.
pub fn populate(store: &CozoStore, graph: &CodeGraph, workspace: Option<&Workspace>) -> Result<()> {
    info!(
        symbols = graph
            .symbol_ids_by_name
            .values()
            .map(|v| v.len())
            .sum::<usize>(),
        files = workspace.map(|w| w.file_count()).unwrap_or(0),
        "cozo populate starting"
    );
    let mut writer = CozoWriter::new();

    let _step3 = info_span!("cozo.populate.tail").entered();
    emit_comments(graph, &mut writer);
    writer.flush(store)?;
    emit_types_and_hierarchy(graph, workspace, &mut writer);
    writer.flush(store)?;

    if let Some(ws) = workspace {
        record_build_meta_files(ws, &mut writer);
    }

    drop(_step3);
    {
        let _fs = info_span!("cozo.populate.flush").entered();
        writer.flush(store)?;
    }
    {
        let _r = info_span!("cozo.populate.call_edge_flush").entered();
        let mut ce_writer = CozoWriter::new();
        resolve_and_emit_call_edges(store, &mut ce_writer)?;
        ce_writer.flush(store)?;
    }
    info!("cozo populate complete");
    Ok(())
}

/// Resolve every `*call_site` to a target `*symbol.id` and emit one
/// `*call_edge{caller_id, callee_id => file_path}` row per resolution.
///
/// Algorithm mirrors the rules that lived inline in the old
/// `test_to_function_map.cozoql`:
///   1. Intra-file: callee_name matches a *symbol{name, file_path, kind}
///      where kind in (function, method, arrow_function, macro) and the
///      callee is not the caller itself.
///   2. Cross-file: caller's file imports a file via *imports; that
///      imported file exports a *symbol{name = callee_name, kind in (...),
///      exported = true}.
///
/// Cost shifts from per-query to once-per-build. Read by any future query
/// that needs resolved call edges.
fn resolve_and_emit_call_edges(store: &CozoStore, writer: &mut CozoWriter) -> Result<()> {
    let _s = info_span!("cozo.populate.call_edge").entered();

    // Two-rule Cozoscript that emits both intra-file and cross-file edges.
    // Mirrors the call_edge rules in find_callers.cozoql / find_callees.cozoql.
    let rows = store.run_query(
        "edge[caller_id, callee_id, file] := \
            *call_site{caller_id, callee_name, file_path: file}, \
            *symbol{id: callee_id, name: callee_name, file_path: file, kind: k}, \
            k in ['function', 'method', 'arrow_function', 'macro'], \
            caller_id != callee_id \
         edge[caller_id, callee_id, file] := \
            *call_site{caller_id, callee_name, file_path: file}, \
            *imports{importer_file_id: file, imported_id: callee_file}, \
            *symbol{id: callee_id, name: callee_name, file_path: callee_file, \
                    kind: k, exported: true}, \
            k in ['function', 'method', 'arrow_function', 'macro'], \
            caller_id != callee_id \
         ?[caller_id, callee_id, file] := edge[caller_id, callee_id, file]",
        BTreeMap::new(),
    )?;

    let mut count = 0usize;
    for row in &rows.rows {
        let caller_id = match &row[0] {
            DataValue::Str(s) => s.as_str(),
            _ => continue,
        };
        let callee_id = match &row[1] {
            DataValue::Str(s) => s.as_str(),
            _ => continue,
        };
        let file = match &row[2] {
            DataValue::Str(s) => s.as_str(),
            _ => continue,
        };
        writer.push_call_edge(caller_id, callee_id, file);
        count += 1;
    }
    eprintln!("[bench] call_edge_count={count}");
    info!(call_edges = count, "cozo call_edge resolution complete");
    Ok(())
}

// `emit_references_facts` and `emit_attrs` removed — those buckets are
// streamed during absorb (see `builder::absorb_file_data`). They never
// land on the graph, so there's nothing to walk here.

/// Issue #13: walk `graph.types` / `graph.param_types` /
/// `graph.returns_types` / `graph.inheritance` and emit the
/// corresponding Cozo rows. Type rows dedupe per `(file_path,
/// display_name)`; parameter / return / inheritance rows resolve their
/// type-id and symbol-id endpoints through the maps built above.
fn emit_types_and_hierarchy(
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    writer: &mut CozoWriter,
) {
    if graph.types.is_empty()
        && graph.param_types.is_empty()
        && graph.returns_types.is_empty()
        && graph.inheritance.is_empty()
        && graph.field_types.is_empty()
        && graph.throws.is_empty()
    {
        return;
    }

    // (file_path, display_name) → type.id, used to fill in
    // parameter.type_id / returns_type.type_id.
    let mut type_id_by_display: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();

    for (file_path, rows) in &graph.types {
        let language = workspace
            .and_then(|ws| ws.file_language(file_path))
            .map(|l| l.as_str())
            .unwrap_or("");
        for row in rows {
            let id = type_id(language, file_path, &row.display_name);
            // Insert-or-keep: rows are pre-deduped per file by the
            // extractor, but the same display_name appearing in two
            // different files produces two distinct ids — that's the
            // ADR-0003 semantics.
            type_id_by_display
                .entry((file_path.clone(), row.display_name.clone()))
                .or_insert_with(|| id.clone());
            writer.push_type(
                &id,
                &row.kind,
                language,
                &row.display_name,
                row.canonical_name.as_deref(),
            );
        }
    }

    // (file_path, name) → list of symbol ids — pre-built during
    // `GraphBuilder::build` from the absorbed symbol records (replaces
    // the Slice-A-and-earlier walk of `graph.nodes`).
    let symbol_ids_by_name = &graph.symbol_ids_by_name;

    // Workspace-wide (name → ids): used to resolve inheritance parents
    // that don't live in the same file as the child.
    let mut symbol_ids_by_global_name: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for ((_, name), ids) in symbol_ids_by_name {
        symbol_ids_by_global_name
            .entry(name.clone())
            .or_default()
            .extend(ids.iter().cloned());
    }

    for (file_path, rows) in &graph.param_types {
        for row in rows {
            let function_id = symbol_id(
                file_path,
                row.function_start_line,
                row.function_start_col,
                &row.function_name,
                row.function_kind,
            );
            let param_id = symbol_id(
                file_path,
                row.parameter_start_line,
                row.parameter_start_col,
                &row.parameter_name,
                SymbolKind::Parameter,
            );
            let language = workspace
                .and_then(|ws| ws.file_language(file_path))
                .map(|l| l.as_str())
                .unwrap_or("");
            let type_id_str = row.type_display_name.as_ref().map(|d| {
                type_id_by_display
                    .get(&(file_path.clone(), d.clone()))
                    .cloned()
                    .unwrap_or_else(|| type_id(language, file_path, d))
            });
            writer.push_parameter(
                &param_id,
                &row.parameter_name,
                &function_id,
                row.position,
                type_id_str.as_deref(),
                row.is_optional,
                row.has_default,
                false,
            );
        }
    }

    for (file_path, rows) in &graph.returns_types {
        for row in rows {
            let function_id = symbol_id(
                file_path,
                row.function_start_line,
                row.function_start_col,
                &row.function_name,
                row.function_kind,
            );
            let language = workspace
                .and_then(|ws| ws.file_language(file_path))
                .map(|l| l.as_str())
                .unwrap_or("");
            let tid = type_id_by_display
                .get(&(file_path.clone(), row.type_display_name.clone()))
                .cloned()
                .unwrap_or_else(|| type_id(language, file_path, &row.type_display_name));
            writer.push_returns_type(&function_id, &tid);
        }
    }

    for (file_path, rows) in &graph.inheritance {
        for row in rows {
            // Resolve child_id. The child is always a workspace symbol
            // (we extracted it from this file). Prefer the in-file
            // candidate that sits at the right line/col; otherwise pick
            // the first same-file match by name.
            let Some(child_id) = pick_symbol_id(
                symbol_ids_by_name,
                file_path,
                &row.child_name,
                Some((row.child_start_line, row.child_start_col)),
            ) else {
                continue;
            };
            // Resolve parent_id. Same-file match wins; then any
            // workspace match by leaf name; otherwise fall back to the
            // canonical_name string (so extends/implements still record
            // the relationship for external parents).
            let parent_leaf = row
                .parent_display_name
                .rsplit("::")
                .next()
                .unwrap_or(&row.parent_display_name)
                .trim_end_matches('>')
                .split('<')
                .next()
                .unwrap_or("")
                .trim();
            let parent_id = pick_symbol_id(symbol_ids_by_name, file_path, parent_leaf, None)
                .or_else(|| {
                    symbol_ids_by_global_name
                        .get(parent_leaf)
                        .and_then(|v| v.first().cloned())
                })
                .or_else(|| row.parent_canonical_name.clone());
            let Some(parent_id) = parent_id else {
                continue;
            };
            match row.kind {
                InheritanceKind::Extends => writer.push_extends(&child_id, &parent_id),
                InheritanceKind::Implements => writer.push_implements(&child_id, &parent_id),
            }
        }
    }

    // Issue #13 followup: `throws` rows. The function's symbol_id is
    // derived from `(file, line, col, name, kind)`; exception_type_id
    // joins through the same per-file display_name → type.id map. If the
    // exception type wasn't emitted by `extract_types` (rare but
    // possible for dynamic forms), we fall back to a synthesised type id.
    for (file_path, rows) in &graph.throws {
        let language = workspace
            .and_then(|ws| ws.file_language(file_path))
            .map(|l| l.as_str())
            .unwrap_or("");
        for row in rows {
            let function_id = symbol_id(
                file_path,
                row.function_start_line,
                row.function_start_col,
                &row.function_name,
                row.function_kind,
            );
            let tid = if let Some(existing) =
                type_id_by_display.get(&(file_path.clone(), row.exception_display_name.clone()))
            {
                existing.clone()
            } else {
                // The exception type wasn't seen by `extract_types` (C#/PHP
                // `throw new X()` runs through `extract_throws` only).
                // Emit a synthetic `named` type row so the JOIN through
                // `*type{}` is non-empty for templates that need it.
                let id = type_id(language, file_path, &row.exception_display_name);
                type_id_by_display.insert(
                    (file_path.clone(), row.exception_display_name.clone()),
                    id.clone(),
                );
                writer.push_type(&id, "named", language, &row.exception_display_name, None);
                id
            };
            writer.push_throws(&function_id, &tid);
        }
    }

    // Issue #14: field_type rows. The field's symbol_id follows the
    // ADR-0002 convention (path|line|col|name|kind); the type_id joins
    // through the per-file display_name → type.id map we built above.
    // Untyped fields don't reach here — the extractor only emits rows
    // when an annotation is present.
    for (file_path, rows) in &graph.field_types {
        let language = workspace
            .and_then(|ws| ws.file_language(file_path))
            .map(|l| l.as_str())
            .unwrap_or("");
        for row in rows {
            let field_symbol_id = symbol_id(
                file_path,
                row.field_start_line,
                row.field_start_col,
                &row.field_name,
                row.field_kind,
            );
            let tid = type_id_by_display
                .get(&(file_path.clone(), row.type_display_name.clone()))
                .cloned()
                .unwrap_or_else(|| type_id(language, file_path, &row.type_display_name));
            writer.push_field_type(&field_symbol_id, &tid);
        }
    }
}

/// Pick a symbol id by `(file_path, name)`. When `hint` is provided,
/// prefer the candidate whose start position matches; otherwise return
/// the first match.
fn pick_symbol_id(
    by_name: &std::collections::HashMap<(String, String), Vec<String>>,
    file_path: &str,
    name: &str,
    _hint: Option<(u32, u32)>,
) -> Option<String> {
    by_name
        .get(&(file_path.to_string(), name.to_string()))
        .and_then(|v| v.first().cloned())
}

fn emit_comments(graph: &CodeGraph, writer: &mut CozoWriter) {
    if graph.comments.is_empty() {
        return;
    }
    // `documents_id` resolution: first symbol with the matching name in
    // the same file wins (extractors emit the symbol the comment
    // textually precedes). Reuses the same `graph.symbol_ids_by_name`
    // map built during absorb; takes `.first()` per (file, name) for
    // the "first wins" semantics.
    let by_name = &graph.symbol_ids_by_name;
    for (file_path, comments) in &graph.comments {
        for (i, c) in comments.iter().enumerate() {
            let id = format!("{}|{}|{}|comment", file_path, c.start_byte, i);
            let documents_id = c.associated_symbol.as_ref().and_then(|name| {
                by_name
                    .get(&(file_path.clone(), name.clone()))
                    .and_then(|v| v.first())
                    .map(|s| s.as_str())
            });
            let is_doc = is_doc_comment(&c.kind, &c.text);
            let todo_kind = detect_todo_kind(&c.text);
            writer.push_comment(
                &id,
                documents_id,
                file_path,
                &c.kind,
                is_doc,
                &c.text,
                todo_kind,
                c.start_byte as i64,
                c.end_byte as i64,
            );
        }
    }
}

/// Heuristic doc-comment classification. Doxygen `/** … */`, Rust `///`
/// and `//!`, Python triple-quoted strings extracted as `kind = "doc"`,
/// JSDoc `/** … */`. Anything else is a non-doc line/block comment.
fn is_doc_comment(kind: &str, text: &str) -> bool {
    if kind == "doc" || kind == "docstring" {
        return true;
    }
    let trimmed = text.trim_start();
    trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("/**")
        || trimmed.starts_with("/*!")
}

fn detect_todo_kind(text: &str) -> Option<&'static str> {
    for kind in ["TODO", "FIXME", "XXX", "HACK"] {
        if text.contains(kind) {
            return Some(match kind {
                "TODO" => "TODO",
                "FIXME" => "FIXME",
                "XXX" => "XXX",
                "HACK" => "HACK",
                _ => unreachable!(),
            });
        }
    }
    None
}

/// Build the canonical String id for a symbol per ADR-0002.
pub fn symbol_id(
    file_path: &str,
    start_line: u32,
    start_col: u32,
    name: &str,
    kind: SymbolKind,
) -> String {
    format!("{file_path}|{start_line}|{start_col}|{name}|{kind}")
}

/// Build the canonical String id for a `type` row per ADR-0003.
///
/// Spec uses blake3 over `language|file_id|display_name`; we use FNV-1a 64
/// (same algorithm as [`crate::cozo::cache_dir_for`]) — same dedup
/// semantics, no extra crate dependency.
pub fn type_id(language: &str, file_path: &str, display_name: &str) -> String {
    // Stable FNV-1a 64 over the canonical concat. Format mirrors the
    // schema's String IDs (no `|` collision with symbol_id since the
    // first segment is a 16-char hex string, not a path).
    let mut h: u64 = 0xcbf29ce484222325;
    for s in [language, "|", file_path, "|", display_name] {
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("type:{h:016x}")
}

// emit_node / emit_edge / symbol_id_of / parent_symbol_id /
// first_call_site_location were all removed in Slice B. Their roles
// (emit *file/*symbol/*span/*imports/*calls rows) are now performed
// by the streaming writer threaded through `GraphBuilder::build` and
// `absorb_file_data`.

/// Wipe every relation populated by [`populate`].
pub fn wipe_workspace_relations(store: &CozoStore) -> Result<()> {
    let wipes: &[&str] = &[
        "?[path] := *file{path} :rm file {path}",
        "?[id] := *symbol{id} :rm symbol {id}",
        "?[entity_id, file_path] := *span{entity_id, file_path} \
         :rm span {entity_id, file_path}",
        "?[caller_id, callee_id] := *calls{caller_id, callee_id} \
         :rm calls {caller_id, callee_id}",
        "?[id] := *call_site{id} :rm call_site {id}",
        "?[child_id, parent_id] := *extends{child_id, parent_id} \
         :rm extends {child_id, parent_id}",
        "?[impl_id, interface_id] := *implements{impl_id, interface_id} \
         :rm implements {impl_id, interface_id}",
        "?[importer_file_id, imported_id] := *imports{importer_file_id, imported_id} \
         :rm imports {importer_file_id, imported_id}",
        "?[file_path, position] := *raw_import{file_path, position} \
         :rm raw_import {file_path, position}",
        "?[id] := *parameter{id} :rm parameter {id}",
        "?[function_id] := *returns_type{function_id} :rm returns_type {function_id}",
        "?[function_id, exception_type_id] := *throws{function_id, exception_type_id} \
         :rm throws {function_id, exception_type_id}",
        "?[symbol_id] := *field_type{symbol_id} :rm field_type {symbol_id}",
        "?[id] := *type{id} :rm type {id}",
        "?[id] := *comment{id} :rm comment {id}",
        "?[path] := *file_classification{path} :rm file_classification {path}",
        "?[file_path, line] := *nolint{file_path, line} :rm nolint {file_path, line}",
        "?[file_path] := *build_meta_files{file_path} :rm build_meta_files {file_path}",
        // Issue #16 — ADR-0005 fact relations.
        "?[id] := *occurrence{id} :rm occurrence {id}",
        "?[id] := *scope{id} :rm scope {id}",
        "?[scope_id, name, start_byte] := *binding{scope_id, name, start_byte} \
         :rm binding {scope_id, name, start_byte}",
        // Issue #15 — per-language attribute relations.
        "?[symbol_id] := *rust_attrs{symbol_id} :rm rust_attrs {symbol_id}",
        "?[symbol_id] := *python_attrs{symbol_id} :rm python_attrs {symbol_id}",
        "?[symbol_id] := *typescript_attrs{symbol_id} \
         :rm typescript_attrs {symbol_id}",
        "?[symbol_id] := *cpp_attrs{symbol_id} :rm cpp_attrs {symbol_id}",
        "?[symbol_id] := *csharp_attrs{symbol_id} :rm csharp_attrs {symbol_id}",
        "?[symbol_id] := *go_attrs{symbol_id} :rm go_attrs {symbol_id}",
        "?[symbol_id] := *php_attrs{symbol_id} :rm php_attrs {symbol_id}",
        "?[symbol_id] := *c_attrs{symbol_id} :rm c_attrs {symbol_id}",
        "?[symbol_id] := *java_attrs{symbol_id} :rm java_attrs {symbol_id}",
    ];
    for stmt in wipes {
        store
            .run_script(stmt, BTreeMap::new())
            .map_err(|e| anyhow::anyhow!("wipe failed for `{stmt}`: {e}"))?;
    }
    Ok(())
}

/// Returns `true` when the stored `build_meta_files` rows match the workspace.
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

pub(crate) fn is_generated_marker(source: &str) -> bool {
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

pub(crate) fn extract_nolints(file_path: &str, source: &str, writer: &mut CozoWriter) {
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
        let store = CozoStore::open_in_memory().expect("open store");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build(&store)
            .expect("build graph");
        populate(&store, &graph, Some(&workspace)).expect("populate");

        // Schema v8: `*calls` empty; raw call sites live in
        // `*call_site`. Derive the edge at query time.
        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *call_site{caller_id, callee_name: callee, file_path: f}, \
                 *symbol{id: callee_id, name: callee, file_path: f, kind: k}, \
                 k in ['function', 'method', 'arrow_function', 'macro'], \
                 caller_id != callee_id, \
                 *symbol{id: caller_id, name: caller}",
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
    fn populate_emits_span_rows_for_symbols() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "fn alpha() {}\n").expect("write");
        let workspace =
            Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
        let store = CozoStore::open_in_memory().expect("open store");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build(&store)
            .expect("build graph");
        populate(&store, &graph, Some(&workspace)).expect("populate");

        let spans = store
            .run_query(
                "?[id, start_line] := *symbol{id, name: 'alpha'}, \
                 *span{entity_id: id, start_line}",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(
            spans.rows.len(),
            1,
            "expected 1 span row, got {:?}",
            spans.rows
        );
    }

    #[test]
    fn populate_classifies_test_barrel_and_generated_files() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mod.rs"), "pub mod inner;\n").expect("write mod");
        std::fs::create_dir(dir.path().join("inner")).expect("dir");
        std::fs::write(dir.path().join("inner").join("mod.rs"), "pub fn ok() {}\n")
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
        let store = CozoStore::open_in_memory().expect("open store");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build(&store)
            .expect("build graph");
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
        let store = CozoStore::open_in_memory().expect("open store");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build(&store)
            .expect("build graph");
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
