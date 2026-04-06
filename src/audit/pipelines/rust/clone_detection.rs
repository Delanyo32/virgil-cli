use std::sync::Arc;

use anyhow::Result;
use tree_sitter::{Point, Query, Tree};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{ancestor_has_kind, is_test_file};

pub struct CloneDetectionPipeline {
    method_query: Arc<Query>,
}

impl CloneDetectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: primitives::compile_method_call_query()?,
        })
    }

    fn message_for_pattern(pattern: &str) -> &'static str {
        match pattern {
            "clone" => ".clone() call — consider borrowing or taking ownership instead",
            "to_owned" => ".to_owned() call — consider borrowing a &str instead",
            "to_string" => {
                ".to_string() on a type that may already be a String — consider borrowing"
            }
            _ => "potential unnecessary clone",
        }
    }

    /// Given a `PatternMatch` line/column (1-based), recover the call node from the tree.
    fn recover_call_node<'a>(
        tree: &'a Tree,
        line: u32,
        column: u32,
    ) -> Option<tree_sitter::Node<'a>> {
        let row = line as usize - 1;
        let col = column as usize - 1;
        let point = Point { row, column: col };
        tree.root_node().descendant_for_point_range(point, point)
    }

    /// Return true if the `to_string` call's receiver is an integer or float literal.
    /// Structure: call_expression → field_expression (value: <receiver>)
    fn receiver_is_numeric_literal(call_node: tree_sitter::Node) -> bool {
        // Walk up from the recovered node to find the call_expression
        let mut current = Some(call_node);
        while let Some(node) = current {
            if node.kind() == "call_expression" {
                // Get the function child (field_expression)
                if let Some(field_expr) = node.child_by_field_name("function") {
                    if field_expr.kind() == "field_expression" {
                        if let Some(receiver) = field_expr.child_by_field_name("value") {
                            let kind = receiver.kind();
                            return kind == "integer_literal" || kind == "float_literal";
                        }
                    }
                }
                return false;
            }
            current = node.parent();
        }
        false
    }
}

impl GraphPipeline for CloneDetectionPipeline {
    fn name(&self) -> &str {
        "clone_detection"
    }

    fn description(&self) -> &str {
        "Detects overuse of .clone(), .to_owned(), and .to_string() that may indicate unnecessary allocations"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        if is_test_file(ctx.file_path) {
            return Vec::new();
        }

        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut findings = Vec::new();

        let method_matches = primitives::find_method_calls(
            tree,
            source,
            &self.method_query,
            &["clone", "to_owned", "to_string"],
        );

        for m in method_matches {
            // Suppress to_string() on integer/float literals (idiomatic: 42.to_string())
            if m.name == "to_string" {
                if let Some(node) = Self::recover_call_node(tree, m.line, m.column) {
                    if Self::receiver_is_numeric_literal(node) {
                        continue;
                    }
                }
            }

            // Determine severity: warning if inside a loop, info otherwise
            let severity = if let Some(node) = Self::recover_call_node(tree, m.line, m.column) {
                if ancestor_has_kind(
                    node,
                    &["for_expression", "while_expression", "loop_expression"],
                ) {
                    "warning"
                } else {
                    "info"
                }
            } else {
                "info"
            };

            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: m.line,
                column: m.column,
                severity: severity.to_string(),
                pipeline: self.name().to_string(),
                pattern: m.name.clone(),
                message: Self::message_for_pattern(&m.name).to_string(),
                snippet: m.text,
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "test.rs")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CloneDetectionPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_clone_calls() {
        let src = r#"
fn example() {
    let a = String::from("hello");
    let b = a.clone();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "clone");
    }

    #[test]
    fn detects_to_owned_and_to_string() {
        let src = r#"
fn example() {
    let a = "hello".to_owned();
    let b = "world".to_string();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);

        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"to_owned"));
        assert!(patterns.contains(&"to_string"));
    }

    #[test]
    fn clean_code_no_findings() {
        let src = r#"
fn example(s: &str) -> usize {
    let a = s.len();
    let b = s.is_empty();
    a + if b { 0 } else { 1 }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn findings_have_correct_metadata() {
        let src = r#"fn main() { let x = vec![1].clone(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "clone_detection");
        assert_eq!(f.pattern, "clone");
        assert_eq!(f.severity, "info");
        assert_eq!(
            f.message,
            ".clone() call — consider borrowing or taking ownership instead"
        );
    }

    #[test]
    fn snippet_captures_full_expression() {
        let src = r#"fn main() { let x = vec![1].clone(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].snippet.contains("vec![1].clone()"));
    }

    #[test]
    fn test_file_excluded() {
        let src = r#"fn f() { let b = a.clone(); }"#;
        let findings = parse_and_check_path(src, "src/tests/helpers.rs");
        assert!(findings.is_empty(), "test files should be excluded");
    }

    #[test]
    fn clone_in_loop_is_warning() {
        let src = r#"
fn example() {
    let items: Vec<String> = vec![];
    for item in &items {
        let b = item.clone();
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].severity,
            "warning",
            "clone in loop should be warning"
        );
    }

    #[test]
    fn clone_outside_loop_is_info() {
        let src = r#"fn example(val: String) -> String { val.clone() }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn arc_clone_scoped_not_flagged() {
        // Arc::clone(&a) is a scoped call (not method call), won't match field_expression query
        let src = r#"
use std::sync::Arc;
fn f(a: Arc<String>) -> Arc<String> { Arc::clone(&a) }
"#;
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "Arc::clone(&a) scoped form should not match method query"
        );
    }

    #[test]
    fn to_string_on_integer_not_flagged() {
        let src = r#"fn f() -> String { 42.to_string() }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "to_string() on integer literal is idiomatic");
    }
}
