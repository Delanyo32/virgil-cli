use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_function_def_query, extract_snippet, find_capture_index, node_text,
};

const LINE_THRESHOLD: usize = 50;
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
                let fn_name = node_text(name_node, source);
                let line_count = body_node.end_position().row - body_node.start_position().row;
                let stmt_count = (0..body_node.named_child_count())
                    .filter_map(|i| body_node.named_child(i))
                    .count();

                let mut reasons = Vec::new();
                if line_count > LINE_THRESHOLD {
                    reasons.push(format!("{line_count} lines (threshold: {LINE_THRESHOLD})"));
                }
                if stmt_count > STATEMENT_THRESHOLD {
                    reasons.push(format!(
                        "{stmt_count} statements (threshold: {STATEMENT_THRESHOLD})"
                    ));
                }

                if !reasons.is_empty() {
                    let start = def_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
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
}
