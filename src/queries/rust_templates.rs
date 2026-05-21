//! Rust-side `--template` handlers.
//!
//! These templates can't be expressed in pure Cozoscript because their
//! inputs require source-of-truth access beyond what's materialised in
//! the fact store. Each handler returns a [`QueryOutput::Findings`]
//! using the audit-shape columns so the CLI formatter treats it
//! uniformly with pure-Cozoscript templates.
//!
//! Currently registered:
//!
//! - **complexity_hotspots** — cyclomatic complexity + function length,
//!   computed on-demand from each function's tree-sitter subtree (issue
//!   #17). The old `start_line`/`end_line` columns on `symbol` are gone
//!   (Datalog migration); spans come from the `span` relation.
//!
//! Old `taint_paths` / `unreleased_resources` handlers are not yet
//! re-implemented — their underlying CFG/taint infra was deleted in the
//! petgraph drop (commit bbf822d). They will land alongside whatever
//! replacement analysis we build atop the new fact store.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use cozo::DataValue;

use crate::cozo::CozoStore;
use crate::storage::workspace::Workspace;

use super::runner::{AuditFinding, QueryOutput};

pub struct Context<'a> {
    pub store: &'a CozoStore,
    pub workspace: &'a Workspace,
    pub params: &'a BTreeMap<String, String>,
}

pub type Handler = fn(&Context<'_>) -> Result<QueryOutput>;

/// Returns a handler for the given template name, or `None` if no
/// Rust-side handler exists (in which case `runner` falls through to the
/// Cozoscript path).
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
/// `*file_classification{is_test: true}`.
///
/// Params:
///   $cc_threshold   — default 10
///   $length_threshold — default 50
fn complexity_hotspots(ctx: &Context<'_>) -> Result<QueryOutput> {
    let cc_threshold = parse_int(ctx.params, "cc_threshold", 10);
    let length_threshold = parse_int(ctx.params, "length_threshold", 50);

    // Pull function/method symbols + their spans, excluding test files.
    // The new schema keeps positional metadata in `span`, not on
    // `symbol`, so we join through it.
    let rows = ctx
        .store
        .run_query(
            "?[name, kind, file, start_line, end_line] := \
             *symbol{id, name, kind, file_path: file}, \
             kind in ['function', 'method', 'arrow_function'], \
             *file_classification{path: file, is_test: false}, \
             *span{entity_id: id, file_path: file, start_line, end_line}",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("failed to query symbols: {e}"))?;

    let mut findings = Vec::new();
    for row in rows.rows {
        let name = match &row[0] {
            DataValue::Str(s) => s.to_string(),
            _ => continue,
        };
        let file = match &row[2] {
            DataValue::Str(s) => s.to_string(),
            _ => continue,
        };
        let start_line = match &row[3] {
            DataValue::Num(cozo::Num::Int(i)) => *i as u32,
            _ => continue,
        };
        let end_line = match &row[4] {
            DataValue::Num(cozo::Num::Int(i)) => *i as u32,
            _ => continue,
        };

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
    findings.sort_by(|a, b| b.line.cmp(&a.line).then(a.file.cmp(&b.file)));
    Ok(QueryOutput::Findings(findings))
}
