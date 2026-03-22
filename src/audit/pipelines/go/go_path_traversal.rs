use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::call_args_reference_params;

use super::primitives::{
    compile_function_decl_query, compile_param_decl_query, compile_selector_call_query,
    extract_snippet, find_capture_index, node_text,
};

pub struct GoPathTraversalPipeline {
    _fn_query: Arc<Query>,
    param_query: Arc<Query>,
    selector_query: Arc<Query>,
}

impl GoPathTraversalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            _fn_query: compile_function_decl_query()?,
            param_query: compile_param_decl_query()?,
            selector_query: compile_selector_call_query()?,
        })
    }

    fn extract_param_names(&self, fn_body: tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        // Walk up from fn_body to the function_declaration to find parameter_list
        if let Some(fn_decl) = fn_body.parent() {
            let mut child_cursor = fn_decl.walk();
            for child in fn_decl.children(&mut child_cursor) {
                if child.kind() == "parameter_list" {
                    let mut param_cursor = QueryCursor::new();
                    let name_idx = find_capture_index(&self.param_query, "param_name");
                    let mut matches =
                        param_cursor.matches(&self.param_query, child, source);
                    while let Some(m) = matches.next() {
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
                }
            }
        }
        names
    }

    fn enclosing_function_body<'a>(
        &self,
        node: tree_sitter::Node<'a>,
    ) -> Option<tree_sitter::Node<'a>> {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_declaration" {
                return parent.child_by_field_name("body");
            }
            current = parent.parent();
        }
        None
    }

    fn call_has_only_literals(call_node: tree_sitter::Node, _source: &[u8]) -> bool {
        // Check if all arguments to the call are string literals
        let mut child_cursor = call_node.walk();
        for child in call_node.children(&mut child_cursor) {
            if child.kind() == "argument_list" {
                let mut arg_cursor = child.walk();
                for arg in child.named_children(&mut arg_cursor) {
                    if arg.kind() != "interpreted_string_literal"
                        && arg.kind() != "raw_string_literal"
                    {
                        return false;
                    }
                }
                return true;
            }
        }
        true
    }
}

impl Pipeline for GoPathTraversalPipeline {
    fn name(&self) -> &str {
        "path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal risks: unvalidated filepath.Join and os.Open with user input"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.selector_query, tree.root_node(), source);

        let pkg_idx = find_capture_index(&self.selector_query, "pkg");
        let method_idx = find_capture_index(&self.selector_query, "method");
        let call_idx = find_capture_index(&self.selector_query, "call");

        while let Some(m) = matches.next() {
            let pkg_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(pkg), Some(method), Some(call)) = (pkg_node, method_node, call_node) {
                let pkg_name = node_text(pkg, source);
                let method_name = node_text(method, source);

                if pkg_name == "filepath" && method_name == "Join" {
                    // Check if call arguments reference function parameters
                    if let Some(fn_body) = self.enclosing_function_body(call) {
                        let param_names = self.extract_param_names(fn_body, source);
                        if call_args_reference_params(call, &param_names, source) {
                            let start = call.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unvalidated_filepath_join".to_string(),
                                message: "filepath.Join in parameterized function — validate path components to prevent traversal".to_string(),
                                snippet: extract_snippet(source, call, 1),
                            });
                        }
                    }
                } else if pkg_name == "os"
                    && (method_name == "Open"
                        || method_name == "Create"
                        || method_name == "OpenFile")
                {
                    // Skip if all arguments are literals (no user input possible)
                    if Self::call_has_only_literals(call, source) {
                        continue;
                    }
                    // Flag only if call arguments reference function parameters
                    if let Some(fn_body) = self.enclosing_function_body(call) {
                        let param_names = self.extract_param_names(fn_body, source);
                        if call_args_reference_params(call, &param_names, source) {
                            let start = call.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unvalidated_file_open".to_string(),
                                message: format!(
                                    "os.{method_name} with non-literal argument in parameterized function — validate path to prevent traversal"
                                ),
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GoPathTraversalPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_filepath_join() {
        let src = r#"package main

import "path/filepath"

func serve(userPath string) {
	filepath.Join("/base", userPath)
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_filepath_join");
    }

    #[test]
    fn detects_os_open() {
        let src = r#"package main

import "os"

func read(name string) {
	os.Open(name)
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_file_open");
    }

    #[test]
    fn ignores_literal_path() {
        let src = r#"package main

import "os"

func f() {
	os.Open("/etc/hosts")
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
