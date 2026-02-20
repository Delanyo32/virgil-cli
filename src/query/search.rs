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
    pub usage_count: i64,
    pub internal_usage: i64,
    pub external_usage: i64,
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
        &[
            "name",
            "kind",
            "file_path",
            "start_line",
            "end_line",
            "is_exported",
            "usage_count",
            "internal_usage",
            "external_usage",
        ],
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
    let safe_query = query.replace('\'', "''");

    let mut conditions = vec![format!("s.name ILIKE '%{}%'", safe_query)];

    if let Some(k) = kind {
        conditions.push(format!("s.kind = '{}'", k.replace('\'', "''")));
    }

    if exported {
        conditions.push("s.is_exported = true".to_string());
    }

    let where_clause = conditions.join(" AND ");

    let sql = if engine.has_imports() {
        format!(
            "SELECT s.name, s.kind, s.file_path, \
             CAST(s.start_line AS INTEGER) as start_line, \
             CAST(s.end_line AS INTEGER) as end_line, \
             s.is_exported, \
             COALESCE(ic.usage_count, 0) AS usage_count, \
             COALESCE(ic.internal_usage, 0) AS internal_usage, \
             COALESCE(ic.external_usage, 0) AS external_usage \
             FROM symbols s \
             LEFT JOIN ( \
                 SELECT imported_name, \
                   COUNT(DISTINCT source_file) AS usage_count, \
                   COUNT(DISTINCT CASE WHEN NOT is_external THEN source_file END) AS internal_usage, \
                   COUNT(DISTINCT CASE WHEN is_external THEN source_file END) AS external_usage \
                 FROM imports GROUP BY imported_name \
             ) ic ON s.name = ic.imported_name AND s.is_exported = true \
             WHERE {} \
             ORDER BY \
               CASE WHEN lower(s.name) = lower('{}') THEN 0 ELSE 1 END, \
               COALESCE(ic.internal_usage, 0) DESC, \
               COALESCE(ic.usage_count, 0) DESC, \
               length(s.name), s.name \
             LIMIT {} OFFSET {}",
            where_clause, safe_query, limit, offset
        )
    } else {
        format!(
            "SELECT s.name, s.kind, s.file_path, \
             CAST(s.start_line AS INTEGER) as start_line, \
             CAST(s.end_line AS INTEGER) as end_line, \
             s.is_exported, \
             0 AS usage_count, \
             0 AS internal_usage, \
             0 AS external_usage \
             FROM symbols s \
             WHERE {} \
             ORDER BY \
               CASE WHEN lower(s.name) = lower('{}') THEN 0 ELSE 1 END, \
               length(s.name), s.name \
             LIMIT {} OFFSET {}",
            where_clause, safe_query, limit, offset
        )
    };

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare search query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SymbolMatch {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                is_exported: row.get(5)?,
                usage_count: row.get(6)?,
                internal_usage: row.get(7)?,
                external_usage: row.get(8)?,
            })
        })
        .context("failed to execute search query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect search results")
}
