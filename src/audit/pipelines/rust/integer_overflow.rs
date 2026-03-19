use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{self, extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

pub struct IntegerOverflowPipeline {
    fn_query: Arc<Query>,
    param_query: Arc<Query>,
    bin_expr_query: Arc<Query>,
}

impl IntegerOverflowPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: primitives::compile_function_item_query()?,
            param_query: primitives::compile_parameter_query()?,
            bin_expr_query: primitives::compile_binary_expression_query()?,
        })
    }
}

impl Pipeline for IntegerOverflowPipeline {
    fn name(&self) -> &str {
        "integer_overflow"
    }

    fn description(&self) -> &str {
        "Detects potential integer overflow: unchecked multiply/add operations in functions taking external input"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.fn_query, "fn_body");
        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_def_idx)
                .map(|c| c.node);

            if let (Some(body), Some(fn_def)) = (body_node, fn_node) {
                // Check if function has parameters
                let has_params = {
                    let mut pc = QueryCursor::new();
                    pc.set_byte_range(fn_def.byte_range());
                    let mut pm = pc.matches(&self.param_query, tree.root_node(), source);
                    pm.next().is_some()
                };

                if !has_params {
                    continue;
                }

                // Find binary expressions within the function body
                let mut bc = QueryCursor::new();
                bc.set_byte_range(body.byte_range());
                let mut bm = bc.matches(&self.bin_expr_query, tree.root_node(), source);

                let bin_expr_idx = find_capture_index(&self.bin_expr_query, "bin_expr");

                while let Some(bm_match) = bm.next() {
                    let expr_node = bm_match
                        .captures
                        .iter()
                        .find(|c| c.index as usize == bin_expr_idx)
                        .map(|c| c.node);

                    if let Some(expr) = expr_node {
                        // Find the operator by iterating non-named children
                        let mut op_text = None;
                        for i in 0..expr.child_count() {
                            if let Some(child) = expr.child(i) {
                                if !child.is_named() {
                                    let text = node_text(child, source);
                                    if text == "*" || text == "+" {
                                        op_text = Some(text);
                                        break;
                                    }
                                }
                            }
                        }

                        if let Some(op) = op_text {
                            let (pattern, severity) = match op {
                                "*" => ("unchecked_multiply", "warning"),
                                "+" => ("unchecked_add", "warning"),
                                _ => continue,
                            };

                            let start = expr.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: severity.to_string(),
                                pipeline: self.name().to_string(),
                                pattern: pattern.to_string(),
                                message: format!(
                                    "arithmetic operation `{op}` on potentially untrusted input may overflow"
                                ),
                                snippet: extract_snippet(source, expr, 1),
                            });
                        }
                    }
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = IntegerOverflowPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_unchecked_multiply() {
        let src = r#"fn calc(n: u32) { let x = n * 2; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unchecked_multiply");
    }

    #[test]
    fn detects_unchecked_add() {
        let src = r#"fn calc(n: u32) { let x = n + 100; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unchecked_add");
    }

    #[test]
    fn ignores_checked_arithmetic() {
        let src = r#"fn calc(n: u32) { let x = n.checked_mul(2); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_functions_without_params() {
        let src = r#"fn internal() { let x = 1 * 2; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
