//! Rust-side `--template` handlers.
//!
//! These templates escape Cozoscript because their inputs cannot be
//! materialised as Cozo facts without duplicating source-of-truth:
//!
//! - **complexity_hotspots** — metrics were deprecated from the schema
//!   (issue 04); the handler re-uses the existing `compute_metric` stage.
//! - **taint_paths** — CFG was deprecated (issue 03); the handler calls
//!   into `src/graph/taint/`.
//! - **unreleased_resources** — same; calls `src/graph/resource.rs`.
//!
//! Each handler returns a [`QueryOutput::Findings`] using the audit-shape
//! columns so the CLI formats it uniformly with pure-Cozoscript
//! templates.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};

use crate::cozo::CozoStore;
use crate::graph::CodeGraph;
use crate::storage::workspace::Workspace;

use super::runner::{AuditFinding, QueryOutput};

pub struct Context<'a> {
    pub store: &'a CozoStore,
    pub graph: &'a CodeGraph,
    pub workspace: &'a Workspace,
    pub params: &'a BTreeMap<String, String>,
}

pub type Handler = fn(&Context<'_>) -> Result<QueryOutput>;

/// Returns a handler for the given template name, or `None` if no Rust-side
/// handler exists (in which case `runner` falls through to the Cozoscript
/// path).
pub fn lookup(name: &str) -> Option<Handler> {
    match name {
        "complexity_hotspots" => Some(complexity_hotspots),
        "taint_paths" => Some(taint_paths),
        "unreleased_resources" => Some(unreleased_resources),
        _ => None,
    }
}

pub fn names() -> &'static [&'static str] {
    &["complexity_hotspots", "taint_paths", "unreleased_resources"]
}

fn parse_int(params: &BTreeMap<String, String>, key: &str, default: i64) -> i64 {
    params.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// complexity_hotspots — flag functions whose cyclomatic complexity OR
/// length exceeds a threshold. Excludes test files via
/// `*file_classification{is_test: true}`.
///
/// Params:
///   $cc_threshold   — default 10
///   $length_threshold — default 50
fn complexity_hotspots(ctx: &Context<'_>) -> Result<QueryOutput> {
    use crate::graph::NodeWeight;
    use crate::models::SymbolKind;

    let cc_threshold = parse_int(ctx.params, "cc_threshold", 10);
    let length_threshold = parse_int(ctx.params, "length_threshold", 50);

    // Pull test files from the store; cheap and avoids re-classifying.
    let test_files = collect_test_files(ctx.store)?;

    let mut findings = Vec::new();
    for node_idx in ctx.graph.graph.node_indices() {
        let NodeWeight::Symbol {
            name,
            kind,
            file_path,
            start_line,
            end_line,
            ..
        } = &ctx.graph.graph[node_idx]
        else {
            continue;
        };
        if !matches!(kind, SymbolKind::Function | SymbolKind::Method | SymbolKind::ArrowFunction)
        {
            continue;
        }
        let file = ctx.graph.symbols.resolve(*file_path).to_string();
        if test_files.contains(&file) {
            continue;
        }

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
        let Some(func_node) = crate::graph::builder::find_node_at_line(
            tree.root_node(),
            *start_line,
            *end_line,
        ) else {
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
        let name = ctx.graph.symbols.resolve(*name).to_string();
        findings.push(AuditFinding {
            file: file.clone(),
            line: *start_line as i64,
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

fn collect_test_files(store: &CozoStore) -> Result<std::collections::HashSet<String>> {
    use cozo::DataValue;
    let rows = store
        .run_query(
            "?[p] := *file_classification{path: p, is_test: true}",
            BTreeMap::new(),
        )
        .map_err(|e| anyhow!("failed to query test files: {e}"))?;
    Ok(rows
        .rows
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(DataValue::Str(s)) => Some(s.to_string()),
            _ => None,
        })
        .collect())
}

/// taint_paths — placeholder handler. The full taint analysis lives in
/// `src/graph/taint/` and is configured via JSON pipelines today. The
/// handler is wired up but currently returns an empty result; a follow-up
/// PR will route `--param source=... --param sink=...` into a one-off
/// `TaintConfig` and call `TaintEngine::analyze_all`.
fn taint_paths(_ctx: &Context<'_>) -> Result<QueryOutput> {
    Ok(QueryOutput::Findings(Vec::new()))
}

/// unreleased_resources — placeholder handler. Wires into
/// `src/graph/resource.rs`; same follow-up note as `taint_paths`.
fn unreleased_resources(_ctx: &Context<'_>) -> Result<QueryOutput> {
    Ok(QueryOutput::Findings(Vec::new()))
}
