use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
pub struct ImportEntry {
    pub source_file: String,
    pub module_specifier: String,
    pub imported_name: String,
    pub local_name: String,
    pub kind: String,
    pub is_type_only: bool,
    pub line: i64,
    pub is_external: bool,
}

pub fn run_imports(
    engine: &QueryEngine,
    module: Option<&str>,
    kind: Option<&str>,
    file: Option<&str>,
    type_only: bool,
    external: bool,
    internal: bool,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_imports() {
        bail!("imports.parquet not found. Re-run `virgil parse` to generate import data.");
    }

    let results = query_imports(engine, module, kind, file, type_only, external, internal, limit)?;
    format_output(
        &results,
        &[
            "source_file",
            "module_specifier",
            "imported_name",
            "local_name",
            "kind",
            "is_type_only",
            "line",
            "is_external",
        ],
        format,
    )
}

fn query_imports(
    engine: &QueryEngine,
    module: Option<&str>,
    kind: Option<&str>,
    file: Option<&str>,
    type_only: bool,
    external: bool,
    internal: bool,
    limit: usize,
) -> Result<Vec<ImportEntry>> {
    let mut conditions: Vec<String> = Vec::new();

    if let Some(m) = module {
        conditions.push(format!(
            "module_specifier LIKE '%{}%'",
            m.replace('\'', "''")
        ));
    }

    if let Some(k) = kind {
        conditions.push(format!("kind = '{}'", k.replace('\'', "''")));
    }

    if let Some(f) = file {
        conditions.push(format!(
            "source_file LIKE '{}%'",
            f.replace('\'', "''")
        ));
    }

    if type_only {
        conditions.push("is_type_only = true".to_string());
    }

    if external {
        conditions.push("is_external = true".to_string());
    }

    if internal {
        conditions.push("is_external = false".to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT source_file, module_specifier, imported_name, local_name, kind, \
         is_type_only, CAST(line AS INTEGER) as line, is_external \
         FROM imports \
         {} \
         ORDER BY source_file, line \
         LIMIT {}",
        where_clause, limit
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare imports query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ImportEntry {
                source_file: row.get(0)?,
                module_specifier: row.get(1)?,
                imported_name: row.get(2)?,
                local_name: row.get(3)?,
                kind: row.get(4)?,
                is_type_only: row.get(5)?,
                line: row.get(6)?,
                is_external: row.get(7)?,
            })
        })
        .context("failed to execute imports query")?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("failed to collect imports results")
}
