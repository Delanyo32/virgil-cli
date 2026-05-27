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
use crate::models::SymbolKind;
use crate::storage::workspace::Workspace;

use super::{DbStore, DbWriter};

/// SQL-staging populate. Comments / types / parameters / returns_types
/// / throws / field_types are now emitted file-locally during absorb,
/// so this phase only:
///   - resolves staged `raw_inheritance` rows into `extends` / `implements`
///   - records workspace file metadata
///   - resolves call sites into `call_edge`
pub fn populate(store: &DbStore, _graph: &CodeGraph, workspace: Option<&Workspace>) -> Result<()> {
    info!(
        files = workspace.map(|w| w.file_count()).unwrap_or(0),
        "db populate starting"
    );
    {
        let _r = info_span!("db.populate.inheritance").entered();
        resolve_inheritance(store)?;
    }
    if let Some(ws) = workspace {
        let mut writer = DbWriter::new();
        record_build_meta_files(ws, &mut writer);
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

/// Resolve every row in `raw_inheritance` to an `extends` / `implements`
/// edge using a SQL JOIN against `symbol` + `imports`. Replaces the
/// per-file Rust loop in the old `emit_types_and_hierarchy` plus the
/// `symbol_ids_by_name` / `symbol_ids_by_global_name` HashMaps held on
/// `CodeGraph`.
///
/// Resolution priority is encoded in the `priority` column inside the
/// CTE — same-file beats imported beats global. `ROW_NUMBER` picks one
/// parent per (child, parent_leaf) so output cardinality matches the
/// prior Rust resolver (one `extends` row per `InheritanceRow`).
fn resolve_inheritance(store: &DbStore) -> Result<()> {
    store.with_conn(|conn| -> Result<()> {
        let sql = "\
            WITH resolved AS ( \
                SELECT \
                    ri.kind AS rel_kind, \
                    child.id AS child_id, \
                    parent.id AS parent_id, \
                    ri.child_start_line, ri.child_start_col, ri.parent_leaf, \
                    CASE \
                        WHEN parent.file_path = ri.file_path THEN 1 \
                        WHEN i.imported_id IS NOT NULL THEN 2 \
                        ELSE 3 \
                    END AS priority \
                FROM raw_inheritance ri \
                JOIN symbol child \
                  ON child.file_path = ri.file_path \
                 AND child.name = ri.child_name \
                 AND child.kind = ri.child_kind \
                JOIN symbol parent \
                  ON parent.name = ri.parent_leaf \
                LEFT JOIN imports i \
                  ON i.importer_file_id = ri.file_path \
                 AND i.imported_id = parent.id \
                WHERE child.id <> parent.id \
            ), \
            ranked AS ( \
                SELECT *, ROW_NUMBER() OVER ( \
                    PARTITION BY child_id, child_start_line, child_start_col, parent_leaf, rel_kind \
                    ORDER BY priority, parent_id \
                ) AS rn \
                FROM resolved \
            ) \
            SELECT rel_kind, child_id, parent_id FROM ranked WHERE rn = 1";

        let mut extends_rows: Vec<(String, String)> = Vec::new();
        let mut implements_rows: Vec<(String, String)> = Vec::new();
        {
            let mut stmt = conn.prepare(sql)?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let rel: String = r.get(0)?;
                let child_id: String = r.get(1)?;
                let parent_id: String = r.get(2)?;
                match rel.as_str() {
                    "extends" => extends_rows.push((child_id, parent_id)),
                    "implements" => implements_rows.push((child_id, parent_id)),
                    _ => {}
                }
            }
        }
        if !extends_rows.is_empty() {
            let mut app = conn.appender("extends")?;
            for (c, p) in extends_rows {
                app.append_row(duckdb::params![c, p])?;
            }
        }
        if !implements_rows.is_empty() {
            let mut app = conn.appender("implements")?;
            for (c, p) in implements_rows {
                app.append_row(duckdb::params![c, p])?;
            }
        }
        // Staging table is no longer needed; drop it to free pages.
        conn.execute("DELETE FROM raw_inheritance", [])?;
        Ok(())
    })
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


pub fn is_doc_comment(kind: &str, text: &str) -> bool {
    if kind == "doc" || kind == "docstring" {
        return true;
    }
    let trimmed = text.trim_start();
    trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("/**")
        || trimmed.starts_with("/*!")
}

pub fn detect_todo_kind(text: &str) -> Option<&'static str> {
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
