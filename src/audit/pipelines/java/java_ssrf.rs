use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_object_creation_query, compile_method_invocation_query,
    extract_snippet, find_capture_index, node_text,
};

pub struct JavaSsrfPipeline {
    creation_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl JavaSsrfPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            creation_query: compile_object_creation_query()?,
            method_query: compile_method_invocation_query()?,
        })
    }
}

impl Pipeline for JavaSsrfPipeline {
    fn name(&self) -> &str {
        "java_ssrf"
    }

    fn description(&self) -> &str {
        "Detects SSRF and open redirect risks: URL creation with user input, sendRedirect with dynamic URL"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_url_creation(tree, source, file_path, &mut findings);
        self.check_redirect(tree, source, file_path, &mut findings);
        findings
    }
}

impl JavaSsrfPipeline {
    fn check_url_creation(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.creation_query, tree.root_node(), source);

        let type_idx = find_capture_index(&self.creation_query, "type_name");
        let args_idx = find_capture_index(&self.creation_query, "args");
        let creation_idx = find_capture_index(&self.creation_query, "creation");

        while let Some(m) = matches.next() {
            let type_node = m.captures.iter().find(|c| c.index as usize == type_idx).map(|c| c.node);
            let args_node = m.captures.iter().find(|c| c.index as usize == args_idx).map(|c| c.node);
            let creation_node = m.captures.iter().find(|c| c.index as usize == creation_idx).map(|c| c.node);

            if let (Some(type_node), Some(args_node), Some(creation_node)) = (type_node, args_node, creation_node) {
                let type_name = node_text(type_node, source);
                if type_name != "URL" && type_name != "URI" {
                    continue;
                }

                // Check if the argument is a variable (not a string literal)
                if let Some(first_arg) = args_node.named_child(0) {
                    if first_arg.kind() != "string_literal" {
                        let args_text = node_text(args_node, source);
                        if args_text.contains('+') || first_arg.kind() == "identifier" {
                            let start = creation_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "ssrf_dynamic_url".to_string(),
                                message: format!(
                                    "new {type_name}() with dynamic input — validate host against allowlist to prevent SSRF"
                                ),
                                snippet: extract_snippet(source, creation_node, 1),
                            });
                        }
                    }
                }
            }
        }
    }

    fn check_redirect(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.method_query, "method_name");
        let args_idx = find_capture_index(&self.method_query, "args");
        let inv_idx = find_capture_index(&self.method_query, "invocation");

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx).map(|c| c.node);
            let args_node = m.captures.iter().find(|c| c.index as usize == args_idx).map(|c| c.node);
            let inv_node = m.captures.iter().find(|c| c.index as usize == inv_idx).map(|c| c.node);

            if let (Some(method_node), Some(args_node), Some(inv_node)) = (method_node, args_node, inv_node) {
                let method_name = node_text(method_node, source);
                if method_name != "sendRedirect" {
                    continue;
                }

                // Check if the argument is a variable (not a string literal)
                if let Some(first_arg) = args_node.named_child(0) {
                    if first_arg.kind() != "string_literal" {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "open_redirect".to_string(),
                            message: "sendRedirect() with dynamic URL — validate against allowlist to prevent open redirect".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = JavaSsrfPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_dynamic_url() {
        let src = r#"class Foo {
    void fetch(String endpoint) {
        URL url = new URL(endpoint);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_dynamic_url");
    }

    #[test]
    fn detects_open_redirect() {
        let src = r#"class Foo {
    void redirect(String url) {
        response.sendRedirect(url);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "open_redirect");
    }

    #[test]
    fn ignores_static_url() {
        let src = r#"class Foo {
    void fetch() {
        URL url = new URL("https://example.com");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_static_redirect() {
        let src = r#"class Foo {
    void redirect() {
        response.sendRedirect("/home");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
