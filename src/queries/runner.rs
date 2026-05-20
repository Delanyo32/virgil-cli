//! Unified runner for Cozoscript / template-based queries.
//!
//! Detects audit-shape output (columns `file, line, severity, pattern,
//! message`) and surfaces it via the [`QueryOutput::Findings`] variant so
//! the CLI formatter can render it as audit findings instead of a raw
//! row table.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use cozo::DataValue;
use serde::Serialize;

use crate::cozo::CozoStore;
use crate::storage::workspace::Workspace;

use super::rust_templates;
use super::templates;

/// Where the Cozoscript came from.
pub enum QuerySource<'a> {
    /// User-supplied inline Cozoscript string.
    Inline(&'a str),
    /// User-supplied path; the runner reads the file.
    File(&'a std::path::Path),
    /// Built-in template by name.
    Template(&'a str),
}

/// Request envelope passed to [`run`].
pub struct QueryRequest<'a> {
    pub source: QuerySource<'a>,
    pub params: Vec<(String, String)>,
    pub store: &'a CozoStore,
    pub workspace: &'a Workspace,
}

/// What the runner returns. Either a plain row table or an audit-finding
/// list when the column shape matches.
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
    /// Any extra columns beyond the canonical five. Keys preserve their
    /// order from the original query.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<(String, serde_json::Value)>,
}

/// Run a query. Dispatches to a Rust-side handler when the template name
/// matches one; otherwise loads the Cozoscript body (inline / file /
/// builtin) and executes it against the store.
pub fn run(req: QueryRequest<'_>) -> Result<QueryOutput> {
    // Rust-side handlers short-circuit before we touch the store.
    if let QuerySource::Template(name) = &req.source {
        if let Some(handler) = rust_templates::lookup(name) {
            let param_map = params_to_btree(&req.params);
            return handler(&rust_templates::Context {
                store: req.store,
                workspace: req.workspace,
                params: &param_map,
            });
        }
    }

    let script = match req.source {
        QuerySource::Inline(s) => s.to_string(),
        QuerySource::File(path) => {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
        }
        QuerySource::Template(name) => templates::load_cozoscript_template(name)
            .ok_or_else(|| anyhow!("unknown template '{name}'"))?
            .to_string(),
    };

    let params = params_to_data_values(&req.params);
    let rows = req
        .store
        .run_query(&script, params)
        .with_context(|| "running cozoscript")?;

    Ok(rows_to_output(rows.headers, rows.rows))
}

/// Convert raw `--param k=v` pairs into `BTreeMap<String, DataValue>` for
/// Cozo. All values bind as strings; templates that need other types use
/// Cozoscript expressions (`to_int($depth)`).
fn params_to_data_values(params: &[(String, String)]) -> BTreeMap<String, DataValue> {
    let mut out = BTreeMap::new();
    for (k, v) in params {
        // Try to bind integers as integers and booleans as booleans —
        // saves users from writing `to_int($x)` everywhere. Falls back to
        // string for anything else.
        if let Ok(i) = v.parse::<i64>() {
            out.insert(k.clone(), DataValue::from(i));
        } else if v == "true" {
            out.insert(k.clone(), DataValue::from(true));
        } else if v == "false" {
            out.insert(k.clone(), DataValue::from(false));
        } else {
            out.insert(k.clone(), DataValue::from(v.as_str()));
        }
    }
    out
}

fn params_to_btree(params: &[(String, String)]) -> BTreeMap<String, String> {
    params.iter().cloned().collect()
}

/// Detect the audit-shape column convention and shape the output rows
/// accordingly. Required columns: `file, line, severity, pattern,
/// message`. Extra columns are preserved as `extras`.
fn rows_to_output(headers: Vec<String>, rows: Vec<Vec<DataValue>>) -> QueryOutput {
    const AUDIT_COLS: &[&str] = &["file", "line", "severity", "pattern", "message"];
    let audit_indices: Option<Vec<usize>> = AUDIT_COLS
        .iter()
        .map(|wanted| headers.iter().position(|h| h == wanted))
        .collect();

    if let Some(indices) = audit_indices {
        let mut findings = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let file = data_value_to_string(&row[indices[0]]).unwrap_or_default();
            let line = data_value_to_i64(&row[indices[1]]).unwrap_or(0);
            let severity = data_value_to_string(&row[indices[2]]).unwrap_or_default();
            let pattern = data_value_to_string(&row[indices[3]]).unwrap_or_default();
            let message = data_value_to_string(&row[indices[4]]).unwrap_or_default();

            let mut extras = Vec::new();
            for (i, h) in headers.iter().enumerate() {
                if AUDIT_COLS.contains(&h.as_str()) {
                    continue;
                }
                extras.push((h.clone(), data_value_to_json(&row[i])));
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
        .map(|r| r.iter().map(data_value_to_json).collect())
        .collect();
    QueryOutput::Rows {
        headers,
        rows: json_rows,
    }
}

fn data_value_to_string(v: &DataValue) -> Option<String> {
    match v {
        DataValue::Str(s) => Some(s.to_string()),
        DataValue::Null => None,
        other => Some(format!("{other:?}")),
    }
}

fn data_value_to_i64(v: &DataValue) -> Option<i64> {
    match v {
        DataValue::Num(n) => match n {
            cozo::Num::Int(i) => Some(*i),
            cozo::Num::Float(f) => Some(*f as i64),
        },
        _ => None,
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
            DataValue::from("src/a.rs"),
            DataValue::from(42i64),
            DataValue::from("warning"),
            DataValue::from("complexity"),
            DataValue::from("too big"),
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
        let rows = vec![vec![DataValue::from("alpha"), DataValue::from(3i64)]];
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
            DataValue::from("a.rs"),
            DataValue::from(1i64),
            DataValue::from("info"),
            DataValue::from("p"),
            DataValue::from("m"),
            DataValue::from(7i64),
        ]];
        let out = super::rows_to_output(headers, rows);
        let QueryOutput::Findings(findings) = out else {
            panic!("expected findings");
        };
        assert_eq!(findings[0].extras.len(), 1);
        assert_eq!(findings[0].extras[0].0, "extra1");
    }
}

pub fn data_value_to_json(v: &DataValue) -> serde_json::Value {
    use serde_json::Value;
    match v {
        DataValue::Null => Value::Null,
        DataValue::Bool(b) => Value::Bool(*b),
        DataValue::Num(n) => match n {
            cozo::Num::Int(i) => Value::from(*i),
            cozo::Num::Float(f) => Value::from(*f),
        },
        DataValue::Str(s) => Value::String(s.to_string()),
        DataValue::List(items) => Value::Array(items.iter().map(data_value_to_json).collect()),
        other => Value::String(format!("{other:?}")),
    }
}
