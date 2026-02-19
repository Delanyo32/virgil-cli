use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct OutlineEntry {
    pub name: String,
    pub kind: String,
    pub start_line: i64,
    pub end_line: i64,
    pub is_exported: bool,
}

#[derive(Debug, Serialize)]
pub struct OutlineImport {
    pub module_specifier: String,
    pub imported_names: String,
    pub kind: String,
    pub is_type_only: bool,
    pub is_external: bool,
}

#[derive(Debug, Serialize)]
pub struct FileOutline {
    pub language: String,
    pub imports: Vec<OutlineImport>,
    pub symbols: Vec<OutlineEntry>,
}

pub fn run_outline(
    engine: &QueryEngine,
    file_path: &str,
    format: &OutputFormat,
) -> Result<String> {
    let language = query_file_language(engine, file_path)?;
    let symbols = query_file_symbols(engine, file_path)?;
    let imports = query_file_imports(engine, file_path)?;

    match format {
        OutputFormat::Json => {
            let outline = FileOutline {
                language: language.clone(),
                imports,
                symbols,
            };
            Ok(serde_json::to_string_pretty(&outline)?)
        }
        _ => {
            let mut out = String::new();
            out.push_str(&format!("File: {}  Language: {}\n\n", file_path, language));

            if !imports.is_empty() {
                out.push_str(&format!("--- Imports ({}) ---\n", imports.len()));
                for imp in &imports {
                    let type_tag = if imp.is_type_only { " (type-only)" } else { "" };
                    out.push_str(&format!(
                        "  {:<30} {}{}\n",
                        imp.module_specifier, imp.imported_names, type_tag
                    ));
                }
                out.push('\n');
            }

            let sym_count = symbols.len();
            out.push_str(&format!("--- Symbols ({}) ---\n", sym_count));
            out.push_str(&format_output(
                &symbols,
                &["name", "kind", "start_line", "end_line", "is_exported"],
                format,
            )?);
            Ok(out)
        }
    }
}

fn query_file_language(engine: &QueryEngine, file_path: &str) -> Result<String> {
    let sql = format!(
        "SELECT language FROM files WHERE path = '{}' LIMIT 1",
        file_path.replace('\'', "''")
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare language query")?;
    let mut rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .context("failed to query file language")?;

    match rows.next() {
        Some(Ok(lang)) => Ok(lang),
        _ => Ok("unknown".to_string()),
    }
}

fn query_file_imports(engine: &QueryEngine, file_path: &str) -> Result<Vec<OutlineImport>> {
    if !engine.has_imports() {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT module_specifier, \
         STRING_AGG(imported_name, ', ' ORDER BY imported_name) AS imported_names, \
         kind, \
         BOOL_OR(is_type_only) AS is_type_only, \
         BOOL_OR(is_external) AS is_external \
         FROM imports \
         WHERE source_file = '{}' \
         GROUP BY module_specifier, kind \
         ORDER BY is_external DESC, module_specifier",
        file_path.replace('\'', "''")
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare outline imports query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(OutlineImport {
                module_specifier: row.get(0)?,
                imported_names: row.get(1)?,
                kind: row.get(2)?,
                is_type_only: row.get(3)?,
                is_external: row.get(4)?,
            })
        })
        .context("failed to execute outline imports query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect outline imports")
}

fn query_file_symbols(engine: &QueryEngine, file_path: &str) -> Result<Vec<OutlineEntry>> {
    let sql = format!(
        "SELECT name, kind, \
         CAST(start_line AS INTEGER) as start_line, \
         CAST(end_line AS INTEGER) as end_line, \
         is_exported \
         FROM symbols \
         WHERE file_path = '{}' \
         ORDER BY start_line",
        file_path.replace('\'', "''")
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare outline query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(OutlineEntry {
                name: row.get(0)?,
                kind: row.get(1)?,
                start_line: row.get(2)?,
                end_line: row.get(3)?,
                is_exported: row.get(4)?,
            })
        })
        .context("failed to execute outline query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect outline results")
}
