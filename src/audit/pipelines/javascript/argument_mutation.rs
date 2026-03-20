use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_function_query, extract_snippet, find_capture_index, node_text};

pub struct ArgumentMutationPipeline {
    func_query: Arc<Query>,
}

impl ArgumentMutationPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            func_query: compile_function_query()?,
        })
    }

    /// Extract parameter names from formal_parameters node.
    fn extract_param_names(params_node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = params_node.walk();
        for child in params_node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    names.push(node_text(child, source).to_string());
                }
                // Destructured params like { a, b } or [a, b] or param with default
                "assignment_pattern" => {
                    if let Some(left) = child.child_by_field_name("left")
                        && left.kind() == "identifier" {
                            names.push(node_text(left, source).to_string());
                        }
                }
                _ => {}
            }
        }
        names
    }

    /// Walk the body looking for assignment_expression where LHS is
    /// member_expression with root object matching a param name.
    fn find_mutations(
        body_node: tree_sitter::Node,
        param_names: &[String],
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        Self::walk_for_mutations(
            body_node,
            param_names,
            source,
            file_path,
            pipeline_name,
            findings,
        );
    }

    fn walk_for_mutations(
        node: tree_sitter::Node,
        param_names: &[String],
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        if node.kind() == "assignment_expression"
            && let Some(lhs) = node.child_by_field_name("left")
                && lhs.kind() == "member_expression" {
                    // Walk to root object of member chain
                    let root = Self::root_object(lhs);
                    if root.kind() == "identifier" {
                        let name = node_text(root, source);
                        if param_names.iter().any(|p| p == name) {
                            let start = node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "argument_mutation".to_string(),
                                message: format!(
                                    "mutating parameter `{name}` — creates hidden side effects for callers"
                                ),
                                snippet: extract_snippet(source, node, 1),
                            });
                            return; // Don't recurse into this assignment's children
                        }
                    }
                }

        // Don't recurse into nested functions — they have their own params
        if node.kind() == "function_declaration"
            || node.kind() == "function_expression"
            || node.kind() == "arrow_function"
        {
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_mutations(
                child,
                param_names,
                source,
                file_path,
                pipeline_name,
                findings,
            );
        }
    }

    fn root_object(node: tree_sitter::Node) -> tree_sitter::Node {
        let mut current = node;
        while let Some(obj) = current.child_by_field_name("object") {
            if obj.kind() == "member_expression" {
                current = obj;
            } else {
                return obj;
            }
        }
        current
    }
}

impl Pipeline for ArgumentMutationPipeline {
    fn name(&self) -> &str {
        "argument_mutation"
    }

    fn description(&self) -> &str {
        "Detects mutation of function parameters — creates hidden side effects"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.func_query, tree.root_node(), source);

        let params_idx = find_capture_index(&self.func_query, "params");
        let body_idx = find_capture_index(&self.func_query, "body");

        while let Some(m) = matches.next() {
            let params_cap = m.captures.iter().find(|c| c.index as usize == params_idx);
            let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);

            if let (Some(params), Some(body)) = (params_cap, body_cap) {
                let param_names = Self::extract_param_names(params.node, source);
                if param_names.is_empty() {
                    continue;
                }

                Self::find_mutations(
                    body.node,
                    &param_names,
                    source,
                    file_path,
                    self.name(),
                    &mut findings,
                );
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ArgumentMutationPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_argument_mutation() {
        let src = "function foo(obj) { obj.name = 'bar'; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "argument_mutation");
        assert!(findings[0].message.contains("obj"));
    }

    #[test]
    fn skips_local_variable_mutation() {
        let src = "function foo(obj) { let local = {}; local.name = 'bar'; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_no_params() {
        let src = "function foo() { x.name = 'bar'; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_deep_mutation() {
        let src = "function foo(config) { config.nested.deep = true; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("config"));
    }

    #[test]
    fn detects_arrow_function_mutation() {
        let src = "const foo = (obj) => { obj.x = 1; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
