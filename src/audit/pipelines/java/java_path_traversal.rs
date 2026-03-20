use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_invocation_with_object_query, compile_object_creation_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct JavaPathTraversalPipeline {
    creation_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl JavaPathTraversalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            creation_query: compile_object_creation_query()?,
            method_query: compile_method_invocation_with_object_query()?,
        })
    }
}

impl Pipeline for JavaPathTraversalPipeline {
    fn name(&self) -> &str {
        "java_path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal risks: File or Paths.get with unvalidated input"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_file_creation(tree, source, file_path, &mut findings);
        self.check_paths_get(tree, source, file_path, &mut findings);
        findings
    }
}

impl JavaPathTraversalPipeline {
    fn check_file_creation(
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
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(args_node), Some(creation_node)) =
                (type_node, args_node, creation_node)
            {
                let type_name = node_text(type_node, source);
                if type_name != "File" {
                    continue;
                }

                let args_text = node_text(args_node, source);
                if args_text.contains('+') {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unvalidated_file_path".to_string(),
                        message: "new File() with concatenated path — validate and normalize to prevent path traversal".to_string(),
                        snippet: extract_snippet(source, creation_node, 1),
                    });
                }
            }
        }
    }

    fn check_paths_get(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let obj_idx = find_capture_index(&self.method_query, "object");
        let method_idx = find_capture_index(&self.method_query, "method_name");
        let args_idx = find_capture_index(&self.method_query, "args");
        let inv_idx = find_capture_index(&self.method_query, "invocation");

        while let Some(m) = matches.next() {
            let obj_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == obj_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(obj_node), Some(method_node), Some(args_node), Some(inv_node)) =
                (obj_node, method_node, args_node, inv_node)
            {
                let obj_name = node_text(obj_node, source);
                let method_name = node_text(method_node, source);

                if obj_name != "Paths" || method_name != "get" {
                    continue;
                }

                // Check if arguments include non-literal values (potential user input)
                let args_text = node_text(args_node, source);
                let has_variable_arg = args_node.named_child_count() > 0 && {
                    let mut has_non_literal = false;
                    for i in 0..args_node.named_child_count() {
                        if let Some(child) = args_node.named_child(i)
                            && child.kind() != "string_literal" {
                                has_non_literal = true;
                                break;
                            }
                    }
                    has_non_literal
                };

                if has_variable_arg && !args_text.contains("normalize") {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unvalidated_paths_get".to_string(),
                        message: "Paths.get() with dynamic argument — normalize and validate against base path".to_string(),
                        snippet: extract_snippet(source, inv_node, 1),
                    });
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
        let pipeline = JavaPathTraversalPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_file_concat() {
        let src = r#"class Foo {
    void read(String name) {
        File f = new File("/uploads/" + name);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_file_path");
    }

    #[test]
    fn detects_paths_get_dynamic() {
        let src = r#"class Foo {
    void read(String name) {
        Paths.get("/uploads", name);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_paths_get");
    }

    #[test]
    fn ignores_static_file_path() {
        let src = r#"class Foo {
    void read() {
        File f = new File("/etc/config.txt");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_file_ops() {
        let src = r#"class Foo {
    void bar() {
        System.out.println("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
