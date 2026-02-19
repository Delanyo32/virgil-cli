use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct DependentEntry {
    pub source_file: String,
    pub imported_name: String,
    pub local_name: String,
    pub kind: String,
    pub line: i64,
}

pub fn run_dependents(
    engine: &QueryEngine,
    file_path: &str,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_imports() {
        bail!("imports.parquet not found. Re-run `virgil parse` to generate import data.");
    }

    let results = query_dependents(engine, file_path)?;
    format_output(
        &results,
        &["source_file", "imported_name", "local_name", "kind", "line"],
        format,
    )
}

fn query_dependents(engine: &QueryEngine, file_path: &str) -> Result<Vec<DependentEntry>> {
    // Match against module_specifier — strip extension and leading "./" for flexible matching.
    // We match where module_specifier contains the file stem.
    let stem = file_path
        .trim_start_matches("./")
        .trim_end_matches(".ts")
        .trim_end_matches(".tsx")
        .trim_end_matches(".js")
        .trim_end_matches(".jsx");

    let sql = format!(
        "SELECT source_file, imported_name, local_name, kind, \
         CAST(line AS INTEGER) as line \
         FROM imports \
         WHERE module_specifier LIKE '%{stem}%' \
         ORDER BY source_file, line",
        stem = stem.replace('\'', "''")
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare dependents query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DependentEntry {
                source_file: row.get(0)?,
                imported_name: row.get(1)?,
                local_name: row.get(2)?,
                kind: row.get(3)?,
                line: row.get(4)?,
            })
        })
        .context("failed to execute dependents query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect dependents results")
}
