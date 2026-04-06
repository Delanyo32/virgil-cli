use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_noqa_suppressed;

use super::primitives::{
    compile_function_def_query, extract_snippet, find_capture_index, node_text,
};

const LINE_THRESHOLD: usize = 50;
const INIT_LINE_THRESHOLD: usize = 100;
const STATEMENT_THRESHOLD: usize = 20;

pub struct GodFunctionsPipeline {
    fn_query: Arc<Query>,
}

impl GodFunctionsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_def_query()?,
        })
    }
}

impl GraphPipeline for GodFunctionsPipeline {
    fn name(&self) -> &str {
        "god_functions"
    }

    fn description(&self) -> &str {
        "Detects functions exceeding size thresholds (>50 lines or >20 statements)"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");
        let fn_body_idx = find_capture_index(&self.fn_query, "fn_body");
        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_body_idx)
                .map(|c| c.node);
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_def_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(def_node)) =
                (name_node, body_node, def_node)
            {
                if is_noqa_suppressed(source, def_node, self.name()) {
                    continue;
                }

                let fn_name = node_text(name_node, source);

                // Exclude docstring lines from line count
                let mut docstring_lines = 0;
                if let Some(first_child) = body_node.named_child(0) {
                    if first_child.kind() == "expression_statement" {
                        if let Some(string_node) = first_child.named_child(0) {
                            if string_node.kind() == "string" {
                                docstring_lines = string_node.end_position().row
                                    - string_node.start_position().row
                                    + 1;
                            }
                        }
                    }
                }

                let raw_line_count =
                    body_node.end_position().row - body_node.start_position().row;
                let line_count = raw_line_count.saturating_sub(docstring_lines);

                // Count statements, excluding docstring expression_statement
                let stmt_count = (0..body_node.named_child_count())
                    .filter_map(|i| body_node.named_child(i))
                    .enumerate()
                    .filter(|(idx, child)| {
                        // Skip first child if it's a docstring
                        if *idx == 0 && child.kind() == "expression_statement" {
                            if let Some(inner) = child.named_child(0) {
                                if inner.kind() == "string" {
                                    return false;
                                }
                            }
                        }
                        true
                    })
                    .count();

                // __init__ gets a higher line threshold
                let effective_line_threshold = if fn_name == "__init__" {
                    INIT_LINE_THRESHOLD
                } else {
                    LINE_THRESHOLD
                };

                let mut reasons = Vec::new();
                if line_count > effective_line_threshold {
                    reasons.push(format!(
                        "{line_count} lines (threshold: {effective_line_threshold})"
                    ));
                }
                if stmt_count > STATEMENT_THRESHOLD {
                    reasons.push(format!(
                        "{stmt_count} statements (threshold: {STATEMENT_THRESHOLD})"
                    ));
                }

                if !reasons.is_empty() {
                    // Severity graduation based on line count
                    let severity = if line_count > 200 {
                        "critical"
                    } else if line_count > 100 {
                        "error"
                    } else {
                        "warning"
                    };

                    let start = def_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "god_function".to_string(),
                        message: format!(
                            "function `{fn_name}` is too large: {} — consider splitting",
                            reasons.join(", ")
                        ),
                        snippet: extract_snippet(source, def_node, 3),
                    });
                }
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GodFunctionsPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_long_function() {
        let body: String = (0..52).map(|i| format!("    x{i} = {i}\n")).collect();
        let src = format!("def big_func():\n{body}");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "god_function");
        assert!(findings[0].message.contains("lines"));
    }

    #[test]
    fn clean_small_function() {
        let src = "def small():\n    x = 1\n    y = 2\n    return x + y\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_many_statements() {
        let body: String = (0..22).map(|i| format!("    x{i} = {i}\n")).collect();
        let src = format!("def many_stmts():\n{body}");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("statements"));
    }

    #[test]
    fn severity_warning_for_medium() {
        let body: String = (0..52).map(|i| format!("    x{i} = {i}\n")).collect();
        let src = format!("def medium_func():\n{body}");
        let findings = parse_and_check(&src);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn severity_error_for_large() {
        let body: String = (0..102).map(|i| format!("    x{i} = {i}\n")).collect();
        let src = format!("def large_func():\n{body}");
        let findings = parse_and_check(&src);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn severity_critical_for_huge() {
        let body: String = (0..202).map(|i| format!("    x{i} = {i}\n")).collect();
        let src = format!("def huge_func():\n{body}");
        let findings = parse_and_check(&src);
        assert_eq!(findings[0].severity, "critical");
    }

    #[test]
    fn init_gets_higher_threshold() {
        // 60 lines: exceeds LINE_THRESHOLD (50) but not INIT_LINE_THRESHOLD (100)
        // Also exceeds STATEMENT_THRESHOLD (20) — but that's separate from __init__ exemption
        // So we test that line threshold is raised: 60 lines would normally trigger but __init__ allows 100
        let body: String = (0..60).map(|i| format!("      self.x{i} = {i}\n")).collect();
        let src = format!("class C:\n  def __init__(self):\n{body}");
        let findings = parse_and_check(&src);
        // Should still get flagged for statement count (60 > 20), but NOT for line count
        assert!(
            findings.iter().all(|f| !f.message.contains("lines")),
            "__init__ with 60 lines should be under higher line threshold"
        );
    }

    #[test]
    fn noqa_suppresses() {
        let body: String = (0..52).map(|i| format!("    x{i} = {i}\n")).collect();
        let src = format!("def big_func():  # noqa\n{body}");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty(), "# noqa should suppress");
    }
}
