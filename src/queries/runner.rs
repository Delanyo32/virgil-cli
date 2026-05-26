//! Unified runner for SQL / template-based queries.
//!
//! Detects audit-shape output (columns `file, line, severity, pattern,
//! message`) and surfaces it via the [`QueryOutput::Findings`] variant so
//! the CLI formatter can render it as audit findings instead of a raw
//! row table.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use duckdb::types::Value;
use serde::Serialize;
use tracing::{debug, info};

use crate::db::DbStore;
use crate::storage::workspace::Workspace;

use super::rust_templates;
use super::templates;

/// Where the SQL came from.
pub enum QuerySource<'a> {
    Inline(&'a str),
    File(&'a std::path::Path),
    Template(&'a str),
}

pub struct QueryRequest<'a> {
    pub source: QuerySource<'a>,
    pub params: Vec<(String, String)>,
    pub store: &'a DbStore,
    pub workspace: &'a Workspace,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum QueryOutput {
    Findings(Vec<AuditFinding>),
    Rows {
        headers: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
    },
}

#[derive(Debug, Serialize)]
pub struct AuditFinding {
    pub file: String,
    pub line: i64,
    pub severity: String,
    pub pattern: String,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<(String, serde_json::Value)>,
}

pub fn run(req: QueryRequest<'_>) -> Result<QueryOutput> {
    let source_kind = match &req.source {
        QuerySource::Inline(_) => "inline",
        QuerySource::File(_) => "file",
        QuerySource::Template(name) => {
            debug!(template = %name, "query template");
            "template"
        }
    };
    let param_keys: Vec<&str> = req.params.iter().map(|(k, _)| k.as_str()).collect();
    debug!(source = source_kind, params = ?param_keys, "running query");

    if let QuerySource::Template(name) = &req.source
        && let Some(handler) = rust_templates::lookup(name)
    {
        let param_map = params_to_btree(&req.params);
        let out = handler(&rust_templates::Context {
            store: req.store,
            workspace: req.workspace,
            params: &param_map,
        })?;
        log_output_summary(&out);
        return Ok(out);
    }

    let script = match req.source {
        QuerySource::Inline(s) => s.to_string(),
        QuerySource::File(path) => {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
        }
        QuerySource::Template(name) => templates::load_sql_template(name)
            .ok_or_else(|| anyhow!("unknown template '{name}'"))?
            .to_string(),
    };

    let params = params_to_values(&req.params);
    let rows = req
        .store
        .run_query(&script, params)
        .with_context(|| "running sql")?;

    let out = rows_to_output(rows.headers, rows.rows);
    log_output_summary(&out);
    Ok(out)
}

fn log_output_summary(out: &QueryOutput) {
    match out {
        QueryOutput::Findings(f) => info!(findings = f.len(), "query complete"),
        QueryOutput::Rows { rows, .. } => info!(rows = rows.len(), "query complete"),
    }
}

/// Convert raw `--param k=v` pairs into typed DuckDB values. Auto-coerce
/// integers and booleans; everything else binds as text.
fn params_to_values(params: &[(String, String)]) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in params {
        if let Ok(i) = v.parse::<i64>() {
            out.insert(k.clone(), Value::BigInt(i));
        } else if v == "true" {
            out.insert(k.clone(), Value::Boolean(true));
        } else if v == "false" {
            out.insert(k.clone(), Value::Boolean(false));
        } else {
            out.insert(k.clone(), Value::Text(v.clone()));
        }
    }
    out
}

fn params_to_btree(params: &[(String, String)]) -> BTreeMap<String, String> {
    params.iter().cloned().collect()
}

fn rows_to_output(headers: Vec<String>, rows: Vec<Vec<Value>>) -> QueryOutput {
    const AUDIT_COLS: &[&str] = &["file", "line", "severity", "pattern", "message"];
    let audit_indices: Option<Vec<usize>> = AUDIT_COLS
        .iter()
        .map(|wanted| headers.iter().position(|h| h == wanted))
        .collect();

    if let Some(indices) = audit_indices {
        let mut findings = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let file = value_to_string(&row[indices[0]]).unwrap_or_default();
            let line = value_to_i64(&row[indices[1]]).unwrap_or(0);
            let severity = value_to_string(&row[indices[2]]).unwrap_or_default();
            let pattern = value_to_string(&row[indices[3]]).unwrap_or_default();
            let message = value_to_string(&row[indices[4]]).unwrap_or_default();

            let mut extras = Vec::new();
            for (i, h) in headers.iter().enumerate() {
                if AUDIT_COLS.contains(&h.as_str()) {
                    continue;
                }
                extras.push((h.clone(), value_to_json(&row[i])));
            }
            findings.push(AuditFinding {
                file,
                line,
                severity,
                pattern,
                message,
                extras,
            });
        }
        return QueryOutput::Findings(findings);
    }

    let json_rows = rows
        .into_iter()
        .map(|r| r.iter().map(value_to_json).collect())
        .collect();
    QueryOutput::Rows {
        headers,
        rows: json_rows,
    }
}

pub fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(format!("{other:?}")),
    }
}

pub fn value_to_i64(v: &Value) -> Option<i64> {
    match v {
        Value::TinyInt(n) => Some(*n as i64),
        Value::SmallInt(n) => Some(*n as i64),
        Value::Int(n) => Some(*n as i64),
        Value::BigInt(n) => Some(*n),
        Value::UTinyInt(n) => Some(*n as i64),
        Value::USmallInt(n) => Some(*n as i64),
        Value::UInt(n) => Some(*n as i64),
        Value::UBigInt(n) => Some(*n as i64),
        Value::Float(f) => Some(*f as i64),
        Value::Double(f) => Some(*f as i64),
        _ => None,
    }
}

pub fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Null => J::Null,
        Value::Boolean(b) => J::Bool(*b),
        Value::TinyInt(n) => J::from(*n),
        Value::SmallInt(n) => J::from(*n),
        Value::Int(n) => J::from(*n),
        Value::BigInt(n) => J::from(*n),
        Value::UTinyInt(n) => J::from(*n),
        Value::USmallInt(n) => J::from(*n),
        Value::UInt(n) => J::from(*n),
        Value::UBigInt(n) => J::from(*n),
        Value::Float(f) => J::from(*f),
        Value::Double(f) => J::from(*f),
        Value::Text(s) => J::String(s.clone()),
        Value::List(items) => J::Array(items.iter().map(value_to_json).collect()),
        other => J::String(format!("{other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_shape_columns_produce_findings() {
        let headers = vec![
            "file".to_string(),
            "line".to_string(),
            "severity".to_string(),
            "pattern".to_string(),
            "message".to_string(),
        ];
        let rows = vec![vec![
            Value::Text("src/a.rs".into()),
            Value::BigInt(42),
            Value::Text("warning".into()),
            Value::Text("complexity".into()),
            Value::Text("too big".into()),
        ]];
        let out = super::rows_to_output(headers, rows);
        let QueryOutput::Findings(findings) = out else {
            panic!("expected findings, got rows");
        };
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, "src/a.rs");
        assert_eq!(findings[0].line, 42);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn non_audit_shape_columns_produce_raw_rows() {
        let headers = vec!["name".to_string(), "count".to_string()];
        let rows = vec![vec![Value::Text("alpha".into()), Value::BigInt(3)]];
        let out = super::rows_to_output(headers, rows);
        assert!(matches!(out, QueryOutput::Rows { .. }));
    }

    #[test]
    fn extra_columns_alongside_audit_shape_are_preserved_in_extras() {
        let headers = vec![
            "file".to_string(),
            "line".to_string(),
            "severity".to_string(),
            "pattern".to_string(),
            "message".to_string(),
            "extra1".to_string(),
        ];
        let rows = vec![vec![
            Value::Text("a.rs".into()),
            Value::BigInt(1),
            Value::Text("info".into()),
            Value::Text("p".into()),
            Value::Text("m".into()),
            Value::BigInt(7),
        ]];
        let out = super::rows_to_output(headers, rows);
        let QueryOutput::Findings(findings) = out else {
            panic!("expected findings");
        };
        assert_eq!(findings[0].extras.len(), 1);
        assert_eq!(findings[0].extras[0].0, "extra1");
    }
}
