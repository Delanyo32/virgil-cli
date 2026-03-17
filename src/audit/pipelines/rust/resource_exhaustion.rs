use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use super::primitives::{self, extract_snippet, find_capture_index, node_text};

pub struct ResourceExhaustionPipeline {
    fn_query: Arc<Query>,
    param_query: Arc<Query>,
    scoped_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl ResourceExhaustionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: primitives::compile_function_item_query()?,
            param_query: primitives::compile_parameter_query()?,
            scoped_query: primitives::compile_scoped_call_query()?,
            method_query: primitives::compile_method_call_query()?,
        })
    }
}

impl Pipeline for ResourceExhaustionPipeline {
    fn name(&self) -> &str {
        "resource_exhaustion"
    }

    fn description(&self) -> &str {
        "Detects resource exhaustion risks: unbounded allocations from user input"
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

                // Find scoped calls containing "with_capacity" within the function body
                {
                    let mut sc = QueryCursor::new();
                    sc.set_byte_range(body.byte_range());
                    let mut sm = sc.matches(&self.scoped_query, tree.root_node(), source);

                    let scoped_fn_idx = find_capture_index(&self.scoped_query, "scoped_fn");
                    let call_idx = find_capture_index(&self.scoped_query, "call");

                    while let Some(sm_match) = sm.next() {
                        let fn_cap = sm_match
                            .captures
                            .iter()
                            .find(|c| c.index as usize == scoped_fn_idx)
                            .map(|c| c.node);
                        let call_cap = sm_match
                            .captures
                            .iter()
                            .find(|c| c.index as usize == call_idx)
                            .map(|c| c.node);

                        if let (Some(fn_n), Some(call_n)) = (fn_cap, call_cap) {
                            let fn_text = node_text(fn_n, source);
                            if fn_text.contains("with_capacity") {
                                let start = call_n.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "unbounded_allocation".to_string(),
                                    message: "allocation with potentially untrusted size may cause resource exhaustion".to_string(),
                                    snippet: extract_snippet(source, call_n, 1),
                                });
                            }
                        }
                    }
                }

                // Find method calls to .reserve() within the function body
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
                            if method == "reserve" {
                                let start = call_n.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "unbounded_allocation".to_string(),
                                    message: "allocation with potentially untrusted size may cause resource exhaustion".to_string(),
                                    snippet: extract_snippet(source, call_n, 1),
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
        let pipeline = ResourceExhaustionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_with_capacity() {
        let src = r#"fn process(n: usize) { let v = Vec::<u8>::with_capacity(n); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_allocation");
    }

    #[test]
    fn detects_reserve() {
        let src = r#"fn process(n: usize) { let mut v = Vec::new(); v.reserve(n); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_allocation");
    }

    #[test]
    fn ignores_no_params() {
        let src = r#"fn internal() { let v = Vec::<u8>::with_capacity(100); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_code_no_findings() {
        let src = r#"fn process(n: usize) { let v = vec![0u8; 1024]; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
