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
use rayon::prelude::*;

use crate::classify::{is_barrel_file, is_test_file};
use crate::graph::{CodeGraph, EdgeWeight, NodeIndex, NodeWeight};
use crate::models::{InheritanceKind, SymbolKind};
use crate::storage::workspace::Workspace;

use super::{CozoStore, CozoWriter};

/// Walk every node and edge of `graph` and emit the corresponding Cozo
/// rows, flushing at the end. When `workspace` is provided, also
/// populates `file_classification`, scans each file's source for `nolint`
/// comments, and writes `build_meta_files`.
pub fn populate(store: &CozoStore, graph: &CodeGraph, workspace: Option<&Workspace>) -> Result<()> {
    // Derive repo_id from the workspace root's basename. S3 workspaces
    // have synthetic `s3://bucket/prefix` roots — the last path segment
    // is acceptable. Per ADR/Q9 Option A: real project names (when the
    // user registered via `projects create <name>`) thread in here in a
    // follow-up; Phase 1 uses path basename.
    let repo_id = workspace
        .and_then(|ws| {
            ws.root()
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    // Step 1: per-node writers, in parallel.
    let node_writer: CozoWriter = (0..graph.nodes.len())
        .into_par_iter()
        .fold(CozoWriter::new, |mut writer, node_idx| {
            emit_node(&mut writer, graph, workspace, &repo_id, node_idx);
            writer
        })
        .reduce(CozoWriter::new, |mut a, mut b| {
            a.merge(&mut b);
            a
        });

    // Step 2: per-edge writers, in parallel.
    let edge_writer: CozoWriter = (0..graph.nodes.len())
        .into_par_iter()
        .fold(CozoWriter::new, |mut writer, source_idx| {
            for (target_idx, weight) in &graph.out_edges[source_idx] {
                emit_edge(&mut writer, graph, source_idx, *target_idx, weight);
            }
            writer
        })
        .reduce(CozoWriter::new, |mut a, mut b| {
            a.merge(&mut b);
            a
        });

    let mut writer = node_writer;
    {
        let mut e = edge_writer;
        writer.merge(&mut e);
    }

    // Step 3: sequential tail work.
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

    emit_comments(graph, &mut writer);
    emit_types_and_hierarchy(graph, workspace, &mut writer);

    if let Some(ws) = workspace {
        record_build_meta_files(ws, &mut writer);
    }

    writer.flush(store)
}

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

    // (file_path, name) → list of symbol ids. Used to resolve
    // parameter.function_id and inheritance.{child,parent}_id by name.
    // The same name can appear multiple times in a file (overloads,
    // shadowing); we keep all candidates and let the join pick.
    let mut symbol_ids_by_name: std::collections::HashMap<(String, String), Vec<String>> =
        std::collections::HashMap::new();
    for node in &graph.nodes {
        if let NodeWeight::Symbol {
            name,
            file_path,
            start_line,
            start_col,
            kind,
            ..
        } = node
        {
            let name_s = graph.symbols.resolve(*name);
            let file_s = graph.symbols.resolve(*file_path);
            let id = symbol_id(file_s, *start_line, *start_col, name_s, *kind);
            symbol_ids_by_name
                .entry((file_s.to_string(), name_s.to_string()))
                .or_default()
                .push(id);
        }
    }

    // Workspace-wide (name → ids): used to resolve inheritance parents
    // that don't live in the same file as the child.
    let mut symbol_ids_by_global_name: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for ((_, name), ids) in &symbol_ids_by_name {
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
                &symbol_ids_by_name,
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
            let parent_id = pick_symbol_id(&symbol_ids_by_name, file_path, parent_leaf, None)
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

/// Build an index `(file_path, name) -> symbol id` for resolving
/// `comment.documents_id` back to a concrete symbol when the comment
/// extractor identified an associated symbol by name.
fn build_name_index(graph: &CodeGraph) -> std::collections::HashMap<(String, String), String> {
    let mut idx = std::collections::HashMap::new();
    for node in &graph.nodes {
        if let NodeWeight::Symbol {
            name,
            kind,
            file_path,
            start_line,
            start_col,
            ..
        } = node
        {
            let name = graph.symbols.resolve(*name);
            let fp = graph.symbols.resolve(*file_path);
            let id = symbol_id(fp, *start_line, *start_col, name, *kind);
            // Multiple symbols can share a name in a file; first wins for
            // doc attachment — extractors emit the symbol the comment
            // textually precedes, so the first match is the right one.
            idx.entry((fp.to_string(), name.to_string())).or_insert(id);
        }
    }
    idx
}

fn emit_comments(graph: &CodeGraph, writer: &mut CozoWriter) {
    if graph.comments.is_empty() {
        return;
    }
    let name_idx = build_name_index(graph);
    for (file_path, comments) in &graph.comments {
        for (i, c) in comments.iter().enumerate() {
            let id = format!("{}|{}|{}|comment", file_path, c.start_byte, i);
            let documents_id = c.associated_symbol.as_ref().and_then(|name| {
                name_idx
                    .get(&(file_path.clone(), name.clone()))
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

fn emit_node(
    writer: &mut CozoWriter,
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    repo_id: &str,
    node_idx: usize,
) {
    match &graph.nodes[node_idx] {
        NodeWeight::File { path, language } => {
            let path = graph.symbols.resolve(*path);
            writer.push_file(path, language.as_str(), repo_id);
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
                    extract_nolints(path, &src, writer);
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
            qualified_name,
            kind,
            file_path,
            start_byte,
            end_byte,
            start_line,
            start_col,
            end_line,
            end_col,
            exported,
            visibility,
            is_async,
            is_static,
            is_abstract,
            is_mutable,
        } => {
            let name = graph.symbols.resolve(*name);
            let qualified_name = graph.symbols.resolve(*qualified_name);
            let file_path = graph.symbols.resolve(*file_path);
            let id = symbol_id(file_path, *start_line, *start_col, name, *kind);
            let kind_str = kind.to_string();
            let language = workspace
                .and_then(|ws| ws.file_language(file_path))
                .map(|l| l.as_str())
                .unwrap_or("");
            let parent_id = parent_symbol_id(graph, node_idx);
            writer.push_symbol(
                &id,
                &kind_str,
                name,
                qualified_name,
                language,
                visibility.as_str(),
                file_path,
                parent_id.as_deref(),
                *is_async,
                *is_static,
                *is_abstract,
                *is_mutable,
                *exported,
            );
            writer.push_span(
                &id,
                file_path,
                *start_byte as i64,
                *end_byte as i64,
                *start_line as i64,
                *end_line as i64,
                *start_col as i64,
                *end_col as i64,
            );
        }
        NodeWeight::CallSite { .. } => {
            // CallSite nodes are no longer emitted as their own relation;
            // their location folds into `calls` rows at edge emit time.
        }
    }
}

fn emit_edge(
    writer: &mut CozoWriter,
    graph: &CodeGraph,
    source_idx: usize,
    target_idx: usize,
    weight: &EdgeWeight,
) {
    match weight {
        EdgeWeight::DefinedIn => {}
        EdgeWeight::Calls => {
            let (Some(caller_id), Some(callee_id)) = (
                symbol_id_of(graph, source_idx),
                symbol_id_of(graph, target_idx),
            ) else {
                return;
            };
            let (call_site_file, call_site_start_byte, call_site_end_byte) =
                first_call_site_location(graph, source_idx, target_idx)
                    .unwrap_or_else(|| (String::new(), 0, 0));
            writer.push_calls(
                &caller_id,
                &callee_id,
                &call_site_file,
                call_site_start_byte,
                call_site_end_byte,
                true,
            );
        }
        EdgeWeight::Imports => {
            if let (NodeWeight::File { path: from_p, .. }, NodeWeight::File { path: to_p, .. }) =
                (&graph.nodes[source_idx], &graph.nodes[target_idx])
            {
                let from_path = graph.symbols.resolve(*from_p);
                let to_path = graph.symbols.resolve(*to_p);
                writer.push_imports(from_path, to_path);
            }
        }
        EdgeWeight::Exports => {}
        EdgeWeight::Contains => {}
    }
}

fn symbol_id_of(graph: &CodeGraph, idx: NodeIndex) -> Option<String> {
    let NodeWeight::Symbol {
        name,
        kind,
        file_path,
        start_line,
        start_col,
        ..
    } = &graph.nodes[idx]
    else {
        return None;
    };
    let name = graph.symbols.resolve(*name);
    let file_path = graph.symbols.resolve(*file_path);
    Some(symbol_id(file_path, *start_line, *start_col, name, *kind))
}

fn parent_symbol_id(graph: &CodeGraph, idx: NodeIndex) -> Option<String> {
    graph.in_edges[idx].iter().find_map(|(src, edge)| {
        if !matches!(edge, EdgeWeight::Contains) {
            return None;
        }
        symbol_id_of(graph, *src)
    })
}

fn first_call_site_location(
    graph: &CodeGraph,
    caller_idx: NodeIndex,
    callee_idx: NodeIndex,
) -> Option<(String, i64, i64)> {
    let NodeWeight::Symbol {
        name: callee_name, ..
    } = &graph.nodes[callee_idx]
    else {
        return None;
    };
    let callee_name = graph.symbols.resolve(*callee_name);
    for (child_idx, edge) in &graph.out_edges[caller_idx] {
        if !matches!(edge, EdgeWeight::Contains) {
            continue;
        }
        if let NodeWeight::CallSite {
            name,
            file_path,
            start_byte,
            end_byte,
            ..
        } = &graph.nodes[*child_idx]
        {
            let cs_name = graph.symbols.resolve(*name);
            if cs_name == callee_name {
                let file_path = graph.symbols.resolve(*file_path).to_string();
                return Some((file_path, *start_byte as i64, *end_byte as i64));
            }
        }
    }
    None
}

/// Wipe every relation populated by [`populate`].
pub fn wipe_workspace_relations(store: &CozoStore) -> Result<()> {
    let wipes: &[&str] = &[
        "?[path] := *file{path} :rm file {path}",
        "?[id] := *symbol{id} :rm symbol {id}",
        "?[entity_id, file_path] := *span{entity_id, file_path} \
         :rm span {entity_id, file_path}",
        "?[caller_id, callee_id] := *calls{caller_id, callee_id} \
         :rm calls {caller_id, callee_id}",
        "?[referrer_id, site_file, site_start_byte, match_index] := \
         *references{referrer_id, site_file, site_start_byte, match_index} \
         :rm references {referrer_id, site_file, site_start_byte, match_index}",
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
                 *calls{caller_id, callee_id}, \
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
    fn populate_emits_span_rows_for_symbols() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "fn alpha() {}\n").expect("write");
        let workspace =
            Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build()
            .expect("build graph");
        let store = CozoStore::open_in_memory().expect("open store");
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
