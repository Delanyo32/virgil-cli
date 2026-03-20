use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_invocation_query, extract_snippet, find_capture_index, node_text,
};

pub struct MagicStringsPipeline {
    invocation_query: Arc<Query>,
}

impl MagicStringsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            invocation_query: compile_method_invocation_query()?,
        })
    }
}

impl Pipeline for MagicStringsPipeline {
    fn name(&self) -> &str {
        "magic_strings"
    }

    fn description(&self) -> &str {
        "Detects .equals() and .equalsIgnoreCase() calls with string literals — extract to constants or enums"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let method_name_idx = find_capture_index(&self.invocation_query, "method_name");
        let args_idx = find_capture_index(&self.invocation_query, "args");
        let invocation_idx = find_capture_index(&self.invocation_query, "invocation");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let invocation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == invocation_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(args_node), Some(invocation_node)) =
                (name_node, args_node, invocation_node)
            {
                let method_name = node_text(name_node, source);
                if method_name != "equals" && method_name != "equalsIgnoreCase" {
                    continue;
                }

                // Check if arguments contain a non-empty string literal
                let has_magic_string = (0..args_node.named_child_count())
                    .filter_map(|i| args_node.named_child(i))
                    .any(|child| {
                        if child.kind() == "string_literal" {
                            let text = node_text(child, source);
                            // Skip empty strings: "" (two quotes only)
                            text.len() > 2
                        } else {
                            false
                        }
                    });

                if has_magic_string {
                    let string_text = (0..args_node.named_child_count())
                        .filter_map(|i| args_node.named_child(i))
                        .find(|child| child.kind() == "string_literal")
                        .map(|n| node_text(n, source))
                        .unwrap_or("\"...\"");

                    let start = invocation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "magic_string".to_string(),
                        message: format!(
                            ".{method_name}({string_text}) uses a magic string — extract to a constant or enum"
                        ),
                        snippet: extract_snippet(source, invocation_node, 3),
                    });
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MagicStringsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_equals_magic_string() {
        let src = "class Foo { void m() { role.equals(\"ADMIN\"); } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_string");
        assert!(findings[0].message.contains("\"ADMIN\""));
    }

    #[test]
    fn detects_equals_ignore_case_magic_string() {
        let src = "class Foo { void m() { s.equalsIgnoreCase(\"admin\"); } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_equals_with_variable() {
        let src = "class Foo { void m() { a.equals(b); } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_equals_empty_string() {
        let src = "class Foo { void m() { s.equals(\"\"); } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
