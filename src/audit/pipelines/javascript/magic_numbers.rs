use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_numeric_literal_query, find_capture_index};
use crate::audit::pipelines::helpers::{is_test_context_js, is_test_file};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", "10", "100", "1000", "256", "512", "1024", "2048", "4096", "8192",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &["lexical_declaration", "switch_case"];

pub struct JsMagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl JsMagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: compile_numeric_literal_query()?,
        })
    }

    fn is_exempt_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk ancestors looking for const declarations
        let mut current = node.parent();
        while let Some(parent) = current {
            if EXEMPT_ANCESTOR_KINDS.contains(&parent.kind()) {
                // Check if it's actually a `const` (not `let`)
                if parent.kind() == "lexical_declaration" {
                    let mut child_cursor = parent.walk();
                    for child in parent.children(&mut child_cursor) {
                        if !child.is_named() {
                            let text = child.utf8_text(source).unwrap_or("");
                            if text == "const" {
                                return true;
                            }
                        }
                    }
                } else {
                    return true;
                }
            }
            current = parent.parent();
        }

        // Skip if this is an array index
        if let Some(parent) = node.parent()
            && parent.kind() == "subscript_expression"
                && let Some(index_child) = parent.named_child(1)
                    && index_child.id() == node.id() {
                        return true;
                    }

        false
    }
}

impl Pipeline for JsMagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals outside const contexts that should be named constants"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        // Skip test files entirely
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.numeric_query, tree.root_node(), source);

        let number_idx = find_capture_index(&self.numeric_query, "number");

        while let Some(m) = matches.next() {
            let num_cap = m.captures.iter().find(|c| c.index as usize == number_idx);

            if let Some(num_cap) = num_cap {
                let value = num_cap.node.utf8_text(source).unwrap_or("");

                if EXCLUDED_VALUES.contains(&value) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node, source) {
                    continue;
                }

                // Skip numbers inside test contexts (describe/it/test blocks)
                if is_test_context_js(num_cap.node, source) {
                    continue;
                }

                let start = num_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "magic_number".to_string(),
                    message: format!(
                        "magic number `{value}` — consider extracting to a named constant for clarity"
                    ),
                    snippet: value.to_string(),
                });
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
        let pipeline = JsMagicNumbersPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_magic_number() {
        let findings = parse_and_check("let x = 42;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_number");
        assert!(findings[0].message.contains("42"));
    }

    #[test]
    fn skips_const_context() {
        let findings = parse_and_check("const x = 1024;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let findings = parse_and_check("let x = 1;\nlet y = 0;\nlet z = 2;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_array_index() {
        let findings = parse_and_check("let x = arr[0];");
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_skip_let_context() {
        let findings = parse_and_check("let x = 42;");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_float_magic_number() {
        let findings = parse_and_check("let pi = 3.14159;");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3.14159"));
    }
}
