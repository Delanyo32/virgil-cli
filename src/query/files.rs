use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::{FileSortField, OutputFormat};
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub language: String,
    pub size_bytes: i64,
    pub line_count: i64,
    pub import_count: i64,
    pub dependent_count: i64,
}

pub fn run_files(
    engine: &QueryEngine,
    language: Option<&str>,
    directory: Option<&str>,
    limit: usize,
    offset: usize,
    sort: &FileSortField,
    format: &OutputFormat,
) -> Result<String> {
    let results = query_files(engine, language, directory, limit, offset, sort)?;
    format_output(
        &results,
        &[
            "path",
            "name",
            "language",
            "size_bytes",
            "line_count",
            "import_count",
            "dependent_count",
        ],
        format,
    )
}

fn sort_column(sort: &FileSortField) -> &'static str {
    match sort {
        FileSortField::Path => "f.path",
        FileSortField::Lines => "f.line_count DESC",
        FileSortField::Size => "f.size_bytes DESC",
        FileSortField::Imports => "import_count DESC",
        FileSortField::Dependents => "dependent_count DESC",
    }
}

fn query_files(
    engine: &QueryEngine,
    language: Option<&str>,
    directory: Option<&str>,
    limit: usize,
    offset: usize,
    sort: &FileSortField,
) -> Result<Vec<FileEntry>> {
    let mut conditions: Vec<String> = Vec::new();

    if let Some(lang) = language {
        conditions.push(format!("f.language = '{}'", lang.replace('\'', "''")));
    }

    if let Some(dir) = directory {
        conditions.push(format!("f.path LIKE '{}%'", dir.replace('\'', "''")));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let order_by = sort_column(sort);

    let sql = if engine.has_imports() {
        format!(
            "SELECT f.path, f.name, f.language, \
             CAST(f.size_bytes AS INTEGER) as size_bytes, \
             CAST(f.line_count AS INTEGER) as line_count, \
             COALESCE(imp.import_count, 0) AS import_count, \
             COALESCE(dep.dependent_count, 0) AS dependent_count \
             FROM files f \
             LEFT JOIN ( \
                 SELECT source_file, COUNT(*) AS import_count \
                 FROM imports GROUP BY source_file \
             ) imp ON f.path = imp.source_file \
             LEFT JOIN ( \
                 SELECT module_specifier, COUNT(DISTINCT source_file) AS dependent_count \
                 FROM imports \
                 WHERE module_specifier LIKE '.%' OR module_specifier LIKE '/%' \
                 GROUP BY module_specifier \
             ) dep ON f.path LIKE '%' || dep.module_specifier || '%' \
             {} \
             ORDER BY {} \
             LIMIT {} OFFSET {}",
            where_clause, order_by, limit, offset
        )
    } else {
        format!(
            "SELECT f.path, f.name, f.language, \
             CAST(f.size_bytes AS INTEGER) as size_bytes, \
             CAST(f.line_count AS INTEGER) as line_count, \
             0 AS import_count, \
             0 AS dependent_count \
             FROM files f \
             {} \
             ORDER BY {} \
             LIMIT {} OFFSET {}",
            where_clause, order_by, limit, offset
        )
    };

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare files query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(FileEntry {
                path: row.get(0)?,
                name: row.get(1)?,
                language: row.get(2)?,
                size_bytes: row.get(3)?,
                line_count: row.get(4)?,
                import_count: row.get(5)?,
                dependent_count: row.get(6)?,
            })
        })
        .context("failed to execute files query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect file results")
}
