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
pub struct FileOutline {
    pub language: String,
    pub symbols: Vec<OutlineEntry>,
}

pub fn run_outline(
    engine: &QueryEngine,
    file_path: &str,
    format: &OutputFormat,
) -> Result<String> {
    let language = query_file_language(engine, file_path)?;
    let symbols = query_file_symbols(engine, file_path)?;

    match format {
        OutputFormat::Json => {
            let outline = FileOutline {
                language: language.clone(),
                symbols,
            };
            Ok(serde_json::to_string_pretty(&outline)?)
        }
        _ => {
            let mut out = String::new();
            out.push_str(&format!("File: {}  Language: {}\n\n", file_path, language));
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
        "SELECT language FROM read_parquet('{}') WHERE path = '{}' LIMIT 1",
        engine.files_parquet(),
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

fn query_file_symbols(engine: &QueryEngine, file_path: &str) -> Result<Vec<OutlineEntry>> {
    let sql = format!(
        "SELECT name, kind, \
         CAST(start_line AS INTEGER) as start_line, \
         CAST(end_line AS INTEGER) as end_line, \
         is_exported \
         FROM read_parquet('{}') \
         WHERE file_path = '{}' \
         ORDER BY start_line",
        engine.symbols_parquet(),
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
