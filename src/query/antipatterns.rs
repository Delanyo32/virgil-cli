use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
struct AntipatternRow {
    file_path: String,
    line: u32,
    issue_type: String,
    category: String,
    severity: String,
    language: String,
    description: String,
    snippet: String,
    symbol_name: String,
}

#[derive(Debug, Serialize)]
pub struct AntipatternsSummary {
    pub total: i64,
    pub type_safety: i64,
    pub error_handling: i64,
    pub correctness: i64,
    pub maintainability: i64,
    pub high_severity: i64,
    pub medium_severity: i64,
    pub low_severity: i64,
}

pub fn run_antipatterns_all(
    engine: &QueryEngine,
    file: Option<&str>,
    category: Option<&str>,
    severity: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_antipatterns() {
        return Ok("No antipatterns data available. Re-create the audit to include antipattern analysis.\n".to_string());
    }

    let mut conditions: Vec<String> = Vec::new();

    if let Some(f) = file {
        conditions.push(format!(
            "file_path LIKE '{}%'",
            f.replace('\'', "''")
        ));
    }
    if let Some(c) = category {
        conditions.push(format!(
            "category = '{}'",
            c.replace('\'', "''")
        ));
    }
    if let Some(s) = severity {
        conditions.push(format!(
            "severity = '{}'",
            s.replace('\'', "''")
        ));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT file_path, line, issue_type, category, severity, language, description, snippet, symbol_name \
         FROM antipatterns{} \
         ORDER BY file_path, line \
         LIMIT {}",
        where_clause, limit
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare antipatterns query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AntipatternRow {
                file_path: row.get(0)?,
                line: row.get(1)?,
                issue_type: row.get(2)?,
                category: row.get(3)?,
                severity: row.get(4)?,
                language: row.get(5)?,
                description: row.get(6)?,
                snippet: row.get(7)?,
                symbol_name: row.get(8)?,
            })
        })
        .context("failed to query antipatterns")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect antipattern rows")?;

    let headers = &[
        "file_path", "line", "issue_type", "category", "severity", "language",
        "description", "snippet", "symbol_name",
    ];
    format_output(&rows, headers, format)
}

pub fn run_type_safety(
    engine: &QueryEngine,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_antipatterns() {
        return Ok("No antipatterns data available. Re-create the audit to include antipattern analysis.\n".to_string());
    }
    run_antipattern_query(engine, "type_safety", file, limit, format)
}

pub fn run_error_handling(
    engine: &QueryEngine,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_antipatterns() {
        return Ok("No antipatterns data available. Re-create the audit to include antipattern analysis.\n".to_string());
    }
    run_antipattern_query(engine, "error_handling", file, limit, format)
}

pub fn run_correctness(
    engine: &QueryEngine,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_antipatterns() {
        return Ok("No antipatterns data available. Re-create the audit to include antipattern analysis.\n".to_string());
    }
    run_antipattern_query(engine, "correctness", file, limit, format)
}

pub fn run_maintainability(
    engine: &QueryEngine,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_antipatterns() {
        return Ok("No antipatterns data available. Re-create the audit to include antipattern analysis.\n".to_string());
    }
    run_antipattern_query(engine, "maintainability", file, limit, format)
}

fn run_antipattern_query(
    engine: &QueryEngine,
    category: &str,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    let mut conditions = vec![format!(
        "category = '{}'",
        category.replace('\'', "''")
    )];

    if let Some(f) = file {
        conditions.push(format!(
            "file_path LIKE '{}%'",
            f.replace('\'', "''")
        ));
    }

    let sql = format!(
        "SELECT file_path, line, issue_type, category, severity, language, description, snippet, symbol_name \
         FROM antipatterns \
         WHERE {} \
         ORDER BY file_path, line \
         LIMIT {}",
        conditions.join(" AND "),
        limit
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare antipatterns query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AntipatternRow {
                file_path: row.get(0)?,
                line: row.get(1)?,
                issue_type: row.get(2)?,
                category: row.get(3)?,
                severity: row.get(4)?,
                language: row.get(5)?,
                description: row.get(6)?,
                snippet: row.get(7)?,
                symbol_name: row.get(8)?,
            })
        })
        .context("failed to query antipatterns")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect antipattern rows")?;

    let headers = &[
        "file_path", "line", "issue_type", "category", "severity", "language",
        "description", "snippet", "symbol_name",
    ];
    format_output(&rows, headers, format)
}

pub fn antipatterns_summary(engine: &QueryEngine) -> Result<AntipatternsSummary> {
    if !engine.has_antipatterns() {
        return Ok(AntipatternsSummary {
            total: 0,
            type_safety: 0,
            error_handling: 0,
            correctness: 0,
            maintainability: 0,
            high_severity: 0,
            medium_severity: 0,
            low_severity: 0,
        });
    }

    let sql = "SELECT \
        COUNT(*), \
        COALESCE(SUM(CASE WHEN category = 'type_safety' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN category = 'error_handling' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN category = 'correctness' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN category = 'maintainability' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN severity = 'high' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN severity = 'medium' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN severity = 'low' THEN 1 ELSE 0 END), 0) \
        FROM antipatterns";

    let mut stmt = engine
        .conn
        .prepare(sql)
        .context("failed to prepare antipatterns summary query")?;
    let summary = stmt
        .query_row([], |row| {
            Ok(AntipatternsSummary {
                total: row.get(0)?,
                type_safety: row.get(1)?,
                error_handling: row.get(2)?,
                correctness: row.get(3)?,
                maintainability: row.get(4)?,
                high_severity: row.get(5)?,
                medium_severity: row.get(6)?,
                low_severity: row.get(7)?,
            })
        })
        .context("failed to query antipatterns summary")?;

    Ok(summary)
}
