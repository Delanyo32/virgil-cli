use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct SymbolMatch {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub is_exported: bool,
}

pub fn run_search(
    engine: &QueryEngine,
    query: &str,
    kind: Option<&str>,
    exported: bool,
    limit: usize,
    offset: usize,
    format: &OutputFormat,
) -> Result<String> {
    let results = query_symbols(engine, query, kind, exported, limit, offset)?;
    format_output(
        &results,
        &["name", "kind", "file_path", "start_line", "end_line", "is_exported"],
        format,
    )
}

fn query_symbols(
    engine: &QueryEngine,
    query: &str,
    kind: Option<&str>,
    exported: bool,
    limit: usize,
    offset: usize,
) -> Result<Vec<SymbolMatch>> {
    let mut conditions = vec![format!(
        "name ILIKE '%{}%'",
        query.replace('\'', "''")
    )];

    if let Some(k) = kind {
        conditions.push(format!("kind = '{}'", k.replace('\'', "''")));
    }

    if exported {
        conditions.push("is_exported = true".to_string());
    }

    let where_clause = conditions.join(" AND ");

    // Order: exact matches first, then shorter names (more specific), then alphabetical
    let sql = format!(
        "SELECT name, kind, file_path, \
         CAST(start_line AS INTEGER) as start_line, \
         CAST(end_line AS INTEGER) as end_line, \
         is_exported \
         FROM symbols \
         WHERE {} \
         ORDER BY \
           CASE WHEN lower(name) = lower('{}') THEN 0 ELSE 1 END, \
           length(name), \
           name \
         LIMIT {} OFFSET {}",
        where_clause,
        query.replace('\'', "''"),
        limit,
        offset
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare search query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SymbolMatch {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                is_exported: row.get(5)?,
            })
        })
        .context("failed to execute search query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect search results")
}
