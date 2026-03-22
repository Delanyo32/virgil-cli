use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{self, extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    all_args_are_literals, call_args_reference_params, is_literal_node_rust,
};

pub struct PathTraversalPipeline {
    fn_query: Arc<Query>,
    method_query: Arc<Query>,
    param_query: Arc<Query>,
}

impl PathTraversalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: primitives::compile_function_item_query()?,
            method_query: primitives::compile_method_call_query()?,
            param_query: primitives::compile_parameter_query()?,
        })
    }
}

impl Pipeline for PathTraversalPipeline {
    fn name(&self) -> &str {
        "path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal risks: unvalidated path join/push with user input"
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
                // Extract parameter names from the function's parameters field
                let param_names: Vec<String> = {
                    let mut pc = QueryCursor::new();
                    pc.set_byte_range(fn_def.byte_range());
                    let name_idx = find_capture_index(&self.param_query, "param_name");
                    let mut pm = pc.matches(&self.param_query, tree.root_node(), source);
                    let mut names = Vec::new();
                    while let Some(m) = pm.next() {
                        if let Some(cap) = m
                            .captures
                            .iter()
                            .find(|c| c.index as usize == name_idx)
                        {
                            let name = node_text(cap.node, source);
                            if !name.is_empty() {
                                names.push(name.to_string());
                            }
                        }
                    }
                    names
                };

                if param_names.is_empty() {
                    continue;
                }

                // Find method calls within the function body
                let mut mc = QueryCursor::new();
                mc.set_byte_range(body.byte_range());
                let mut mm = mc.matches(&self.method_query, tree.root_node(), source);
                let name_idx = find_capture_index(&self.method_query, "method_name");
                let call_idx = find_capture_index(&self.method_query, "call");

                while let Some(im) = mm.next() {
                    let name_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == name_idx)
                        .map(|c| c.node);
                    let call_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(name), Some(call)) = (name_node, call_node) {
                        let method_name = node_text(name, source);
                        let call_text = node_text(call, source);

                        // Only flag if this looks like a path operation
                        // Heuristic: receiver text contains "path", "dir", or "file" (case-insensitive)
                        let call_lower = call_text.to_lowercase();
                        let is_path_op = call_lower.contains("path")
                            || call_lower.contains("dir")
                            || call_lower.contains("file");

                        if is_path_op {
                            // Skip if all arguments to the call are safe literals
                            if let Some(args_node) = call.child_by_field_name("arguments") {
                                if all_args_are_literals(args_node, is_literal_node_rust) {
                                    continue;
                                }
                            }

                            // Only flag if call arguments actually reference a function parameter
                            if !call_args_reference_params(call, &param_names, source) {
                                continue;
                            }

                            let (pattern, msg) = match method_name {
                                "join" => (
                                    "unvalidated_path_join",
                                    "path join with potentially untrusted input may allow directory traversal",
                                ),
                                "push" => (
                                    "unvalidated_path_push",
                                    "path push with potentially untrusted input may allow directory traversal",
                                ),
                                _ => continue,
                            };

                            let start = call.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: pattern.to_string(),
                                message: msg.to_string(),
                                snippet: extract_snippet(source, call, 1),
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
        let pipeline = PathTraversalPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_path_join() {
        let src = r#"
fn serve(input: &str) {
    let path = std::path::PathBuf::from("/base");
    path.join(input);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_path_join");
    }

    #[test]
    fn detects_path_push() {
        let src = r#"
fn serve(input: &str) {
    let mut file_path = std::path::PathBuf::new();
    file_path.push(input);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_path_push");
    }

    #[test]
    fn ignores_non_path_join() {
        let src = r#"
fn f(x: &str) {
    let v = vec!["a"];
    v.join(",");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_no_params() {
        let src = r#"
fn internal() {
    let path = std::path::PathBuf::from("/base");
    path.join("fixed");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
