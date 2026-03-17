use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::java_primitives::{
    compile_assignment_query, extract_snippet, find_capture_index, node_text,
};

pub struct StringConcatInLoopsPipeline {
    assignment_query: Arc<Query>,
}

impl StringConcatInLoopsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            assignment_query: compile_assignment_query()?,
        })
    }
}

impl Pipeline for StringConcatInLoopsPipeline {
    fn name(&self) -> &str {
        "string_concat_in_loops"
    }

    fn description(&self) -> &str {
        "Detects string concatenation with += inside loops — use StringBuilder instead"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.assignment_query, tree.root_node(), source);

        let assign_idx = find_capture_index(&self.assignment_query, "assign");
        let rhs_idx = find_capture_index(&self.assignment_query, "rhs");

        while let Some(m) = matches.next() {
            let assign_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == assign_idx)
                .map(|c| c.node);
            let rhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == rhs_idx)
                .map(|c| c.node);

            if let (Some(assign_node), Some(rhs_node)) = (assign_node, rhs_node) {
                // Check if operator is +=
                let assign_text = node_text(assign_node, source);
                if !assign_text.contains("+=") {
                    continue;
                }

                // Check if inside a loop
                if !is_inside_loop(assign_node) {
                    continue;
                }

                // Heuristic: check if RHS involves strings
                let is_string_op = contains_string_literal(rhs_node, source)
                    || rhs_node.kind() == "binary_expression";

                if is_string_op {
                    let start = assign_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "string_concat_in_loop".to_string(),
                        message:
                            "string concatenation with += inside a loop — use StringBuilder instead"
                                .to_string(),
                        snippet: extract_snippet(source, assign_node, 3),
                    });
                }
            }
        }

        findings
    }
}

fn is_inside_loop(node: tree_sitter::Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" => {
                return true
            }
            "method_declaration" | "constructor_declaration" | "class_declaration" => return false,
            _ => parent = p.parent(),
        }
    }
    false
}

fn contains_string_literal(node: tree_sitter::Node, source: &[u8]) -> bool {
    if node.kind() == "string_literal" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_string_literal(child, source) {
            return true;
        }
    }
    false
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
        let pipeline = StringConcatInLoopsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_string_concat_in_for_loop() {
        let src = r#"
class Foo {
    void m() {
        String s = "";
        for (int i = 0; i < 10; i++) {
            s += "item";
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "string_concat_in_loop");
    }

    #[test]
    fn clean_outside_loop() {
        let src = r#"
class Foo {
    void m() {
        String s = "";
        s += "x";
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_in_while_loop() {
        let src = r#"
class Foo {
    void m() {
        String s = "";
        int i = 0;
        while (i < 10) {
            s += "item";
            i++;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
