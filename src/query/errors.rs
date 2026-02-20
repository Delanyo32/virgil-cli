use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct ErrorEntry {
    pub file_path: String,
    pub file_name: String,
    pub extension: String,
    pub language: String,
    pub error_type: String,
    pub error_message: String,
    pub size_bytes: i64,
}

pub fn run_errors(
    engine: &QueryEngine,
    error_type: Option<&str>,
    language: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_errors() {
        bail!("errors.parquet not found. Re-run `virgil parse` to generate error data.");
    }

    let results = query_errors(engine, error_type, language, limit)?;
    format_output(
        &results,
        &[
            "file_path",
            "file_name",
            "extension",
            "language",
            "error_type",
            "error_message",
            "size_bytes",
        ],
        format,
    )
}

fn query_errors(
    engine: &QueryEngine,
    error_type: Option<&str>,
    language: Option<&str>,
    limit: usize,
) -> Result<Vec<ErrorEntry>> {
    let mut conditions: Vec<String> = Vec::new();

    if let Some(et) = error_type {
        conditions.push(format!("error_type = '{}'", et.replace('\'', "''")));
    }

    if let Some(lang) = language {
        conditions.push(format!("language = '{}'", lang.replace('\'', "''")));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT file_path, file_name, extension, language, error_type, error_message, \
         CAST(size_bytes AS BIGINT) as size_bytes \
         FROM errors \
         {} \
         ORDER BY file_path \
         LIMIT {}",
        where_clause, limit
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare errors query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ErrorEntry {
                file_path: row.get(0)?,
                file_name: row.get(1)?,
                extension: row.get(2)?,
                language: row.get(3)?,
                error_type: row.get(4)?,
                error_message: row.get(5)?,
                size_bytes: row.get(6)?,
            })
        })
        .context("failed to execute errors query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect errors results")
}
