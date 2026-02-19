use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct DepEntry {
    pub module_specifier: String,
    pub imported_name: String,
    pub local_name: String,
    pub kind: String,
    pub is_type_only: bool,
    pub line: i64,
}

pub fn run_deps(
    engine: &QueryEngine,
    file_path: &str,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_imports() {
        bail!("imports.parquet not found. Re-run `virgil parse` to generate import data.");
    }

    let results = query_deps(engine, file_path)?;
    format_output(
        &results,
        &[
            "module_specifier",
            "imported_name",
            "local_name",
            "kind",
            "is_type_only",
            "line",
        ],
        format,
    )
}

fn query_deps(engine: &QueryEngine, file_path: &str) -> Result<Vec<DepEntry>> {
    let sql = format!(
        "SELECT module_specifier, imported_name, local_name, kind, is_type_only, \
         CAST(line AS INTEGER) as line \
         FROM imports \
         WHERE source_file = '{}' \
         ORDER BY line",
        file_path.replace('\'', "''")
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare deps query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DepEntry {
                module_specifier: row.get(0)?,
                imported_name: row.get(1)?,
                local_name: row.get(2)?,
                kind: row.get(3)?,
                is_type_only: row.get(4)?,
                line: row.get(5)?,
            })
        })
        .context("failed to execute deps query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect deps results")
}
