use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{COMMON_ALLOWED_NUMBERS, is_test_file};

use super::primitives::{compile_numeric_literal_query, find_capture_index};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", "10", "100", "1000", "256", "512", "1024", "2048", "4096", "8192",
    "16384", "32768", "65536", "0xFF", "0xff", "0x80", "0xFFFF", "0xffff",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &[
    "const_declaration",
    "const_spec",
    "case_clause",
    "expression_case",
    "call_expression",
];

pub struct GoMagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl GoMagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: compile_numeric_literal_query()?,
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

        // Skip if this is an index expression
        if let Some(parent) = node.parent()
            && parent.kind() == "index_expression"
            && let Some(index_child) = parent.named_child(1)
            && index_child.id() == node.id()
        {
            return true;
        }

        false
    }
}

impl Pipeline for GoMagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals outside const contexts that should be named constants"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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

                if EXCLUDED_VALUES.contains(&value) || COMMON_ALLOWED_NUMBERS.contains(&value) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node) {
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GoMagicNumbersPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_magic_number() {
        let src = "package main\nfunc main() {\n\tx := 42 + y\n\t_ = x\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_number");
        assert!(findings[0].message.contains("42"));
    }

    #[test]
    fn skips_const_context() {
        let src = "package main\nconst maxWorkers = 32\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let src = "package main\nfunc main() {\n\tx := 1\n\ty := 0\n\tz := 2\n\t_ = x + y + z\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_index_expression() {
        let src = "package main\nfunc main() {\n\tx := arr[0]\n\t_ = x\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_float_magic_number() {
        let src = "package main\nfunc main() {\n\tpi := 3.14159\n\t_ = pi\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3.14159"));
    }
}
