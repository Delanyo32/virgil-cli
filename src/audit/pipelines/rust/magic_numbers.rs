use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{is_test_context_rust, is_test_file};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", // Common powers of 2 and sizes
    "10", "100", "1000", "256", "512", "1024", "2048", "4096", "8192", "16384", "32768", "65536",
    // Common hex masks
    "0xFF", "0xff", "0x80", "0xFFFF", "0xffff", "0xFF00", "0xff00", "0x00", "0x01", "0x02",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &[
    "const_item",
    "static_item",
    "enum_variant",
    "attribute_item",
    "match_arm",
    "range_expression",
    "macro_invocation",
];

pub struct MagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl MagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: primitives::compile_numeric_literal_query()?,
        })
    }

    fn is_exempt_context(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if EXEMPT_ANCESTOR_KINDS.contains(&parent.kind()) {
                return true;
            }
            current = parent.parent();
        }

        // Skip if this is an index expression (arr[0])
        if let Some(parent) = node.parent()
            && parent.kind() == "index_expression" {
                // Check if this literal is the index (second child)
                if let Some(index_child) = parent.named_child(1)
                    && index_child.id() == node.id() {
                        return true;
                    }
            }

        false
    }
}

impl Pipeline for MagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals not in const/static/enum contexts that should be named constants"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        // Skip test files entirely
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.numeric_query, tree.root_node(), source);

        let number_idx = self
            .numeric_query
            .capture_names()
            .iter()
            .position(|n| *n == "number")
            .unwrap();

        while let Some(m) = matches.next() {
            let num_node = m.captures.iter().find(|c| c.index as usize == number_idx);

            if let Some(num_cap) = num_node {
                let value = num_cap.node.utf8_text(source).unwrap_or("");

                if EXCLUDED_VALUES.contains(&value) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node) {
                    continue;
                }

                // Skip numbers inside test contexts
                if is_test_context_rust(num_cap.node, source) {
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MagicNumbersPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_magic_number() {
        let src = r#"
fn example() {
    let x = 9999;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_number");
        assert!(findings[0].message.contains("9999"));
    }

    #[test]
    fn skips_const_context() {
        let src = r#"
const N: usize = 1024;
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let src = r#"
fn example() {
    let x = 1;
    let y = 0;
    let z = 2;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_index_expression() {
        let src = r#"
fn example() {
    let x = arr[0];
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_static_context() {
        let src = r#"
static MAX: usize = 512;
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_float_magic_number() {
        let src = r#"
fn example() {
    let pi = 3.14159;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3.14159"));
    }
}
