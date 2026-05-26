//! Populate a [`DbStore`] from a finished [`CodeGraph`].
//!
//! 1:1 port of `src/cozo/from_code_graph.rs`. Reads the same scratch
//! state off `CodeGraph`, emits the same logical rows, runs the same
//! parallel rayon call-edge resolver — just against DuckDB tables and
//! the duckdb-rs `Value` type instead of cozo `DataValue`.
//!
//! Skipped vs the cozo version:
//! - `wipe_workspace_relations` — no incremental refresh in the
//!   experiment (Q6 decision).
//! - `is_warm_compatible` — `DbStore::open_persistent` already
//!   version-checks via `build_meta`; warm reuse is "fresh = false".

use std::collections::HashMap;

use anyhow::Result;
use duckdb::types::Value;
use rayon::prelude::*;
use tracing::{info, info_span};

use crate::graph::CodeGraph;
use crate::models::{InheritanceKind, SymbolKind};
use crate::storage::workspace::Workspace;

use super::{DbStore, DbWriter};

/// See `cozo::from_code_graph::populate` for the design contract.
pub fn populate(store: &DbStore, graph: &CodeGraph, workspace: Option<&Workspace>) -> Result<()> {
    info!(
        symbols = graph
            .symbol_ids_by_name
            .values()
            .map(|v| v.len())
            .sum::<usize>(),
        files = workspace.map(|w| w.file_count()).unwrap_or(0),
        "db populate starting"
    );
    let mut writer = DbWriter::new();

    let _step3 = info_span!("db.populate.tail").entered();
    emit_comments(graph, &mut writer);
    writer.flush(store)?;
    emit_types_and_hierarchy(graph, workspace, &mut writer);
    writer.flush(store)?;

    if let Some(ws) = workspace {
        record_build_meta_files(ws, &mut writer);
    }

    drop(_step3);
    {
        let _fs = info_span!("db.populate.flush").entered();
        writer.flush(store)?;
    }
    {
        let _r = info_span!("db.populate.call_edge_flush").entered();
        let mut ce_writer = DbWriter::new();
        resolve_and_emit_call_edges(store, &mut ce_writer)?;
        ce_writer.flush(store)?;
    }
    info!("db populate complete");
    Ok(())
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    }
}

fn bool_of(v: &Value) -> bool {
    matches!(v, Value::Boolean(true))
}

/// See `cozo::from_code_graph::resolve_and_emit_call_edges` for the
/// algorithm — this is the same code path against DuckDB tables.
fn resolve_and_emit_call_edges(store: &DbStore, writer: &mut DbWriter) -> Result<()> {
    let _s = info_span!("db.populate.call_edge").entered();

    let cs_rows = store.run_query(
        "SELECT id, caller_id, callee_name, file_path FROM call_site",
        std::collections::BTreeMap::new(),
    )?;
    let mut call_sites: Vec<(String, String, String)> = Vec::with_capacity(cs_rows.rows.len());
    for row in &cs_rows.rows {
        let Some(caller_id) = text_of(&row[1]) else {
            continue;
        };
        let Some(callee_name) = text_of(&row[2]) else {
            continue;
        };
        let Some(file_path) = text_of(&row[3]) else {
            continue;
        };
        call_sites.push((caller_id, callee_name, file_path));
    }

    let sym_rows = store.run_query(
        "SELECT id, name, file_path, kind, exported FROM symbol \
         WHERE kind IN ('function', 'method', 'arrow_function', 'macro')",
        std::collections::BTreeMap::new(),
    )?;
    let mut intra: HashMap<(String, String), Vec<(String, String)>> =
        HashMap::with_capacity(sym_rows.rows.len());
    let mut exports: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for row in &sym_rows.rows {
        let Some(symbol_id) = text_of(&row[0]) else {
            continue;
        };
        let Some(name) = text_of(&row[1]) else {
            continue;
        };
        let Some(file_path) = text_of(&row[2]) else {
            continue;
        };
        let Some(kind) = text_of(&row[3]) else {
            continue;
        };
        let exported = bool_of(&row[4]);
        intra
            .entry((file_path.clone(), name.clone()))
            .or_default()
            .push((kind.clone(), symbol_id.clone()));
        if exported {
            exports
                .entry((file_path, name))
                .or_default()
                .push((kind, symbol_id));
        }
    }

    let imp_rows = store.run_query(
        "SELECT importer_file_id, imported_id FROM imports",
        std::collections::BTreeMap::new(),
    )?;
    let mut imports_by_importer: HashMap<String, Vec<String>> =
        HashMap::with_capacity(imp_rows.rows.len());
    for row in &imp_rows.rows {
        let Some(importer) = text_of(&row[0]) else {
            continue;
        };
        let Some(imported) = text_of(&row[1]) else {
            continue;
        };
        imports_by_importer
            .entry(importer)
            .or_default()
            .push(imported);
    }

    const CHUNK_SIZE: usize = 1024;
    let resolved: Vec<(String, String, String)> = call_sites
        .par_chunks(CHUNK_SIZE)
        .flat_map(|chunk| {
            let mut local: Vec<(String, String, String)> = Vec::new();
            for (caller_id, callee_name, file) in chunk {
                if let Some(candidates) = intra.get(&(file.clone(), callee_name.clone())) {
                    for (_kind, callee_id) in candidates {
                        if caller_id != callee_id {
                            local.push((caller_id.clone(), callee_id.clone(), file.clone()));
                        }
                    }
                }
                if let Some(imported_files) = imports_by_importer.get(file) {
                    for imported_file in imported_files {
                        if let Some(candidates) =
                            exports.get(&(imported_file.clone(), callee_name.clone()))
                        {
                            for (_kind, callee_id) in candidates {
                                if caller_id != callee_id {
                                    local.push((
                                        caller_id.clone(),
                                        callee_id.clone(),
                                        file.clone(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            local
        })
        .collect();

    let count = resolved.len();
    for (caller_id, callee_id, file) in resolved {
        writer.push_call_edge(&caller_id, &callee_id, &file);
    }
    eprintln!("[bench] call_edge_count={count}");
    info!(call_edges = count, "db call_edge resolution complete");
    Ok(())
}

fn emit_types_and_hierarchy(
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    writer: &mut DbWriter,
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

    let mut type_id_by_display: HashMap<(String, String), String> = HashMap::new();

    for (file_path, rows) in &graph.types {
        let language = workspace
            .and_then(|ws| ws.file_language(file_path))
            .map(|l| l.as_str())
            .unwrap_or("");
        for row in rows {
            let id = type_id(language, file_path, &row.display_name);
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

    let symbol_ids_by_name = &graph.symbol_ids_by_name;

    let mut symbol_ids_by_global_name: HashMap<String, Vec<String>> = HashMap::new();
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
            let Some(child_id) = pick_symbol_id(
                symbol_ids_by_name,
                file_path,
                &row.child_name,
                Some((row.child_start_line, row.child_start_col)),
            ) else {
                continue;
            };
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

fn pick_symbol_id(
    by_name: &HashMap<(String, String), Vec<String>>,
    file_path: &str,
    name: &str,
    _hint: Option<(u32, u32)>,
) -> Option<String> {
    by_name
        .get(&(file_path.to_string(), name.to_string()))
        .and_then(|v| v.first().cloned())
}

fn emit_comments(graph: &CodeGraph, writer: &mut DbWriter) {
    if graph.comments.is_empty() {
        return;
    }
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

/// Canonical String id for a symbol per ADR-0002.
pub fn symbol_id(
    file_path: &str,
    start_line: u32,
    start_col: u32,
    name: &str,
    kind: SymbolKind,
) -> String {
    format!("{file_path}|{start_line}|{start_col}|{name}|{kind}")
}

/// Canonical String id for a `type` row per ADR-0003.
pub fn type_id(language: &str, file_path: &str, display_name: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for s in [language, "|", file_path, "|", display_name] {
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("type:{h:016x}")
}

fn record_build_meta_files(workspace: &Workspace, writer: &mut DbWriter) {
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

pub(crate) fn extract_nolints(file_path: &str, source: &str, writer: &mut DbWriter) {
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
