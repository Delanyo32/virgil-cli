//! Rust-side `--template` handlers.
//!
//! These templates can't be expressed in pure SQL because their inputs
//! require source-of-truth access beyond what's materialised in the
//! fact store. Each handler returns a [`QueryOutput::Findings`] using
//! the audit-shape columns so the CLI formatter treats it uniformly
//! with pure-SQL templates.
//!
//! Currently registered:
//!
//! - **complexity_hotspots** — cyclomatic complexity + function length,
//!   computed on-demand from each function's tree-sitter subtree.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use duckdb::types::Value;

use crate::db::DbStore;
use crate::storage::workspace::Workspace;

use super::runner::{AuditFinding, QueryOutput, value_to_i64, value_to_string};

pub struct Context<'a> {
    pub store: &'a DbStore,
    pub workspace: &'a Workspace,
    pub params: &'a BTreeMap<String, String>,
}

pub type Handler = fn(&Context<'_>) -> Result<QueryOutput>;

pub fn lookup(name: &str) -> Option<Handler> {
    match name {
        "complexity_hotspots" => Some(complexity_hotspots),
        _ => None,
    }
}

pub fn names() -> &'static [&'static str] {
    &["complexity_hotspots"]
}

fn parse_int(params: &BTreeMap<String, String>, key: &str, default: i64) -> i64 {
    params
        .get(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// complexity_hotspots — flag functions whose cyclomatic complexity OR
/// length exceeds a threshold. Excludes test files via
/// `file_classification.is_test = true`.
fn complexity_hotspots(ctx: &Context<'_>) -> Result<QueryOutput> {
    let cc_threshold = parse_int(ctx.params, "cc_threshold", 10);
    let length_threshold = parse_int(ctx.params, "length_threshold", 50);

    let rows = ctx
        .store
        .run_query(
            "SELECT s.name, s.kind, s.file_path, sp.start_line, sp.end_line \
             FROM symbol s \
             JOIN file_classification fc ON fc.path = s.file_path AND fc.is_test = false \
             JOIN span sp ON sp.entity_id = s.id AND sp.file_path = s.file_path \
             WHERE s.kind IN ('function', 'method', 'arrow_function')",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("failed to query symbols: {e}"))?;

    let mut findings = Vec::new();
    for row in rows.rows {
        let Some(name) = value_to_string(&row[0]) else {
            continue;
        };
        let Some(file) = value_to_string(&row[2]) else {
            continue;
        };
        let Some(start_line) = value_to_i64(&row[3]) else {
            continue;
        };
        let Some(end_line) = value_to_i64(&row[4]) else {
            continue;
        };
        let start_line = start_line as u32;
        let end_line = end_line as u32;

        let Some(lang) = ctx.workspace.file_language(&file) else {
            continue;
        };
        let Some(source) = ctx.workspace.read_file(&file) else {
            continue;
        };
        let mut parser = match crate::parser::create_parser(lang) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let Some(tree) = parser.parse(source.as_bytes(), None) else {
            continue;
        };
        let Some(func_node) =
            crate::graph::builder::find_node_at_line(tree.root_node(), start_line, end_line)
        else {
            continue;
        };
        let body_field = crate::graph::metrics::body_field_for_language(lang);
        let Some(body) = func_node.child_by_field_name(body_field) else {
            continue;
        };
        let config = crate::graph::metrics::control_flow_config_for_language(lang);
        let cc = crate::graph::metrics::compute_cyclomatic(body, &config, source.as_bytes()) as i64;
        let (length, _) = crate::graph::metrics::count_function_lines(body);
        let length = length as i64;

        if cc < cc_threshold && length < length_threshold {
            continue;
        }

        let severity = if cc >= 20 {
            "error"
        } else if cc >= 10 {
            "warning"
        } else {
            "info"
        };
        findings.push(AuditFinding {
            file: file.clone(),
            line: start_line as i64,
            severity: severity.to_string(),
            pattern: "high_complexity".to_string(),
            message: format!("{name}: cyclomatic={cc}, length={length}"),
            extras: vec![
                ("cyclomatic".to_string(), serde_json::Value::from(cc)),
                ("length".to_string(), serde_json::Value::from(length)),
            ],
        });
    }
    // Suppress unused-import warning when Value isn't used directly.
    let _: Option<Value> = None;
    findings.sort_by(|a, b| b.line.cmp(&a.line).then(a.file.cmp(&b.file)));
    Ok(QueryOutput::Findings(findings))
}
