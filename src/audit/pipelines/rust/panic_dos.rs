use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{self, extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

const NARROWING_TYPES: &[&str] = &["i8", "i16", "i32", "u8", "u16", "u32"];

pub struct PanicDosPipeline {
    fn_query: Arc<Query>,
    param_query: Arc<Query>,
    method_query: Arc<Query>,
    index_query: Arc<Query>,
    cast_query: Arc<Query>,
}

impl PanicDosPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: primitives::compile_function_item_query()?,
            param_query: primitives::compile_parameter_query()?,
            method_query: primitives::compile_method_call_query()?,
            index_query: primitives::compile_index_expression_query()?,
            cast_query: primitives::compile_type_cast_expression_query()?,
        })
    }
}

impl Pipeline for PanicDosPipeline {
    fn name(&self) -> &str {
        "panic_dos"
    }

    fn description(&self) -> &str {
        "Detects denial-of-service via panics: unwrap on untrusted input, unbounded indexing, narrowing casts"
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

                // Find .unwrap() and .expect() calls within the function body
                {
                    let mut mc = QueryCursor::new();
                    mc.set_byte_range(body.byte_range());
                    let mut mm = mc.matches(&self.method_query, tree.root_node(), source);

                    let method_name_idx = find_capture_index(&self.method_query, "method_name");
                    let call_idx = find_capture_index(&self.method_query, "call");

                    while let Some(mm_match) = mm.next() {
                        let name_cap = mm_match
                            .captures
                            .iter()
                            .find(|c| c.index as usize == method_name_idx)
                            .map(|c| c.node);
                        let call_cap = mm_match
                            .captures
                            .iter()
                            .find(|c| c.index as usize == call_idx)
                            .map(|c| c.node);

                        if let (Some(name_n), Some(call_n)) = (name_cap, call_cap) {
                            let method = node_text(name_n, source);
                            if method == "unwrap" || method == "expect" {
                                let start = call_n.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "unwrap_untrusted".to_string(),
                                    message: "unwrap/expect on untrusted input may cause denial of service".to_string(),
                                    snippet: extract_snippet(source, call_n, 1),
                                });
                            }
                        }
                    }
                }

                // Find index expressions within the function body
                {
                    let mut ic = QueryCursor::new();
                    ic.set_byte_range(body.byte_range());
                    let mut im = ic.matches(&self.index_query, tree.root_node(), source);

                    let index_expr_idx = find_capture_index(&self.index_query, "index_expr");

                    while let Some(im_match) = im.next() {
                        let expr_cap = im_match
                            .captures
                            .iter()
                            .find(|c| c.index as usize == index_expr_idx)
                            .map(|c| c.node);

                        if let Some(expr_n) = expr_cap {
                            let start = expr_n.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unbounded_slice".to_string(),
                                message: "direct indexing on untrusted input may panic".to_string(),
                                snippet: extract_snippet(source, expr_n, 1),
                            });
                        }
                    }
                }

                // Find narrowing type casts within the function body
                {
                    let mut cc = QueryCursor::new();
                    cc.set_byte_range(body.byte_range());
                    let mut cm = cc.matches(&self.cast_query, tree.root_node(), source);

                    let cast_type_idx = find_capture_index(&self.cast_query, "cast_type");
                    let cast_expr_idx = find_capture_index(&self.cast_query, "cast_expr");

                    while let Some(cm_match) = cm.next() {
                        let type_cap = cm_match
                            .captures
                            .iter()
                            .find(|c| c.index as usize == cast_type_idx)
                            .map(|c| c.node);
                        let expr_cap = cm_match
                            .captures
                            .iter()
                            .find(|c| c.index as usize == cast_expr_idx)
                            .map(|c| c.node);

                        if let (Some(type_n), Some(expr_n)) = (type_cap, expr_cap) {
                            let cast_type = node_text(type_n, source);
                            if NARROWING_TYPES.contains(&cast_type) {
                                let start = expr_n.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "narrowing_cast".to_string(),
                                    message:
                                        "narrowing cast may truncate and cause unexpected behavior"
                                            .to_string(),
                                    snippet: extract_snippet(source, expr_n, 1),
                                });
                            }
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
        let pipeline = PanicDosPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_unwrap_in_parameterized_fn() {
        let src = r#"fn handle(input: &str) { let n: u32 = input.parse().unwrap(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unwrap_untrusted");
    }

    #[test]
    fn detects_unbounded_slice() {
        let src = r#"fn get(data: &[u8], idx: usize) { let x = data[idx]; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_slice");
    }

    #[test]
    fn detects_narrowing_cast() {
        let src = r#"fn convert(n: u64) { let x = n as u32; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "narrowing_cast");
    }

    #[test]
    fn ignores_no_params() {
        let src = r#"fn internal() { let x = Some(1).unwrap(); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
