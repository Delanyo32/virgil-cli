use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct CallerEntry {
    pub source_file: String,
    pub module_specifier: String,
    pub local_name: String,
    pub kind: String,
    pub is_type_only: bool,
    pub line: i64,
    pub is_external: bool,
}

pub fn run_callers(
    engine: &QueryEngine,
    symbol_name: &str,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_imports() {
        bail!("imports.parquet not found. Re-run `virgil parse` to generate import data.");
    }

    let results = query_callers(engine, symbol_name, limit)?;
    format_output(
        &results,
        &[
            "source_file",
            "module_specifier",
            "local_name",
            "kind",
            "is_type_only",
            "line",
            "is_external",
        ],
        format,
    )
}

fn query_callers(
    engine: &QueryEngine,
    symbol_name: &str,
    limit: usize,
) -> Result<Vec<CallerEntry>> {
    let safe_name = symbol_name.replace('\'', "''");

    let sql = format!(
        "SELECT source_file, module_specifier, local_name, kind, is_type_only, \
         CAST(line AS INTEGER) as line, is_external \
         FROM imports \
         WHERE imported_name ILIKE '%{safe_name}%' \
         ORDER BY \
           CASE WHEN lower(imported_name) = lower('{safe_name}') THEN 0 ELSE 1 END, \
           is_external ASC, \
           source_file, line \
         LIMIT {limit}",
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare callers query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CallerEntry {
                source_file: row.get(0)?,
                module_specifier: row.get(1)?,
                local_name: row.get(2)?,
                kind: row.get(3)?,
                is_type_only: row.get(4)?,
                line: row.get(5)?,
                is_external: row.get(6)?,
            })
        })
        .context("failed to execute callers query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect callers results")
}
