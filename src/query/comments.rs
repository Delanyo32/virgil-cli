use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct CommentEntry {
    pub file_path: String,
    pub text: String,
    pub kind: String,
    pub start_line: i64,
    pub end_line: i64,
    pub associated_symbol: Option<String>,
    pub associated_symbol_kind: Option<String>,
}

pub fn run_comments(
    engine: &QueryEngine,
    file: Option<&str>,
    kind: Option<&str>,
    documented: bool,
    symbol: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_comments() {
        bail!("comments.parquet not found. Re-run `virgil parse` to generate comment data.");
    }

    let results = query_comments(engine, file, kind, documented, symbol, limit)?;
    format_output(
        &results,
        &[
            "file_path",
            "text",
            "kind",
            "start_line",
            "end_line",
            "associated_symbol",
            "associated_symbol_kind",
        ],
        format,
    )
}

fn query_comments(
    engine: &QueryEngine,
    file: Option<&str>,
    kind: Option<&str>,
    documented: bool,
    symbol: Option<&str>,
    limit: usize,
) -> Result<Vec<CommentEntry>> {
    let mut conditions: Vec<String> = Vec::new();

    if let Some(f) = file {
        conditions.push(format!(
            "file_path LIKE '{}%'",
            f.replace('\'', "''")
        ));
    }

    if let Some(k) = kind {
        conditions.push(format!("kind = '{}'", k.replace('\'', "''")));
    }

    if documented {
        conditions.push("associated_symbol IS NOT NULL".to_string());
    }

    if let Some(s) = symbol {
        conditions.push(format!(
            "associated_symbol LIKE '%{}%'",
            s.replace('\'', "''")
        ));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT file_path, text, kind, \
         CAST(start_line AS INTEGER) as start_line, \
         CAST(end_line AS INTEGER) as end_line, \
         associated_symbol, associated_symbol_kind \
         FROM comments \
         {} \
         ORDER BY file_path, start_line \
         LIMIT {}",
        where_clause, limit
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare comments query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CommentEntry {
                file_path: row.get(0)?,
                text: row.get(1)?,
                kind: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                associated_symbol: row.get(5)?,
                associated_symbol_kind: row.get(6)?,
            })
        })
        .context("failed to execute comments query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect comments results")
}
