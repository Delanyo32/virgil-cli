use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::query::db::QueryEngine;
use crate::query::format::format_output;

#[derive(Debug, Serialize)]
struct SecurityRow {
    file_path: String,
    line: u32,
    description: String,
    snippet: String,
    symbol_name: String,
}

#[derive(Debug, Serialize)]
pub struct SecuritySummary {
    pub unsafe_calls: i64,
    pub string_risks: i64,
    pub hardcoded_secrets: i64,
    pub total: i64,
    pub high_severity: i64,
    pub medium_severity: i64,
}

pub fn run_unsafe_calls(
    engine: &QueryEngine,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_security() {
        return Ok("No security data available. Re-create the audit to include security analysis.\n".to_string());
    }
    run_security_query(engine, "unsafe_call", file, limit, format)
}

pub fn run_string_risks(
    engine: &QueryEngine,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_security() {
        return Ok("No security data available. Re-create the audit to include security analysis.\n".to_string());
    }
    run_security_query(engine, "string_risk", file, limit, format)
}

pub fn run_hardcoded_secrets(
    engine: &QueryEngine,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    if !engine.has_security() {
        return Ok("No security data available. Re-create the audit to include security analysis.\n".to_string());
    }
    run_security_query(engine, "hardcoded_secret", file, limit, format)
}

fn run_security_query(
    engine: &QueryEngine,
    issue_type: &str,
    file: Option<&str>,
    limit: usize,
    format: &OutputFormat,
) -> Result<String> {
    let mut conditions = vec![format!(
        "issue_type = '{}'",
        issue_type.replace('\'', "''")
    )];

    if let Some(f) = file {
        conditions.push(format!(
            "file_path LIKE '{}%'",
            f.replace('\'', "''")
        ));
    }

    let sql = format!(
        "SELECT file_path, line, description, snippet, symbol_name \
         FROM security \
         WHERE {} \
         ORDER BY file_path, line \
         LIMIT {}",
        conditions.join(" AND "),
        limit
    );

    let mut stmt = engine
        .conn
        .prepare(&sql)
        .context("failed to prepare security query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SecurityRow {
                file_path: row.get(0)?,
                line: row.get(1)?,
                description: row.get(2)?,
                snippet: row.get(3)?,
                symbol_name: row.get(4)?,
            })
        })
        .context("failed to query security")?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to collect security rows")?;

    let headers = &["file_path", "line", "description", "snippet", "symbol_name"];
    format_output(&rows, headers, format)
}

pub fn security_summary(engine: &QueryEngine) -> Result<SecuritySummary> {
    if !engine.has_security() {
        return Ok(SecuritySummary {
            unsafe_calls: 0,
            string_risks: 0,
            hardcoded_secrets: 0,
            total: 0,
            high_severity: 0,
            medium_severity: 0,
        });
    }

    let sql = "SELECT \
        COALESCE(SUM(CASE WHEN issue_type = 'unsafe_call' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN issue_type = 'string_risk' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN issue_type = 'hardcoded_secret' THEN 1 ELSE 0 END), 0), \
        COUNT(*), \
        COALESCE(SUM(CASE WHEN severity = 'high' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN severity = 'medium' THEN 1 ELSE 0 END), 0) \
        FROM security";

    let mut stmt = engine
        .conn
        .prepare(sql)
        .context("failed to prepare security summary query")?;
    let summary = stmt
        .query_row([], |row| {
            Ok(SecuritySummary {
                unsafe_calls: row.get(0)?,
                string_risks: row.get(1)?,
                hardcoded_secrets: row.get(2)?,
                total: row.get(3)?,
                high_severity: row.get(4)?,
                medium_severity: row.get(5)?,
            })
        })
        .context("failed to query security summary")?;

    Ok(summary)
}
