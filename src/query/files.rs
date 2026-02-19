use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub language: String,
    pub size_bytes: i64,
    pub line_count: i64,
}

pub fn run_files(
    engine: &QueryEngine,
    language: Option<&str>,
    directory: Option<&str>,
    limit: usize,
    offset: usize,
    format: &OutputFormat,
) -> Result<String> {
    let results = query_files(engine, language, directory, limit, offset)?;
    format_output(
        &results,
        &["path", "name", "language", "size_bytes", "line_count"],
        format,
    )
}

fn query_files(
    engine: &QueryEngine,
    language: Option<&str>,
    directory: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<Vec<FileEntry>> {
    let mut conditions: Vec<String> = Vec::new();

    if let Some(lang) = language {
        conditions.push(format!("language = '{}'", lang.replace('\'', "''")));
    }

    if let Some(dir) = directory {
        conditions.push(format!(
            "path LIKE '{}%'",
            dir.replace('\'', "''")
        ));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT path, name, language, \
         CAST(size_bytes AS INTEGER) as size_bytes, \
         CAST(line_count AS INTEGER) as line_count \
         FROM files \
         {} \
         ORDER BY path \
         LIMIT {} OFFSET {}",
        where_clause,
        limit,
        offset
    );

    let mut stmt = engine.conn.prepare(&sql).context("failed to prepare files query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(FileEntry {
                path: row.get(0)?,
                name: row.get(1)?,
                language: row.get(2)?,
                size_bytes: row.get(3)?,
                line_count: row.get(4)?,
            })
        })
        .context("failed to execute files query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect file results")
}
