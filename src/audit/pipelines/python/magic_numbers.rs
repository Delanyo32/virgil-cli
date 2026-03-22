use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    ancestor_has_kind, is_test_context_python, is_test_file, COMMON_ALLOWED_NUMBERS,
};

use super::primitives::{compile_numeric_literal_query, find_capture_index, node_text};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "-1", "0.0", "1.0", "10", "100", "1000", "256", "512", "1024", "2048", "4096",
    "8192",
];

pub struct PythonMagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl PythonMagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: compile_numeric_literal_query()?,
        })
    }

    fn is_exempt_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            match parent.kind() {
                // ALL_CAPS assignment — constant definition
                "assignment" => {
                    if let Some(lhs) = parent.child_by_field_name("left") {
                        let name = node_text(lhs, source);
                        if name == name.to_uppercase() && name.chars().any(|c| c.is_alphabetic()) {
                            return true;
                        }
                    }
                }
                // Subscript index position (arr[3])
                "subscript" => {
                    // Check if the number is the index (second named child)
                    if let Some(index) = parent.named_child(1)
                        && index.id() == node.id()
                    {
                        return true;
                    }
                }
                // keyword argument (func(timeout=30))
                "keyword_argument" => {
                    return true;
                }
                // default parameter value
                "default_parameter" | "typed_default_parameter" => {
                    return true;
                }
                _ => {}
            }
            current = parent.parent();
        }
        false
    }
}

impl Pipeline for PythonMagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals outside constant contexts that should be named constants"
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

                if EXCLUDED_VALUES.contains(&value) || COMMON_ALLOWED_NUMBERS.contains(&value) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node, source) {
                    continue;
                }

                // Skip numbers inside assert statements
                if ancestor_has_kind(num_cap.node, &["assert_statement"]) {
                    continue;
                }

                // Skip numbers inside test contexts
                if is_test_context_python(num_cap.node, source, file_path) {
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PythonMagicNumbersPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_magic_number() {
        let src = "x = 9999\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("9999"));
    }

    #[test]
    fn skips_constant_assignment() {
        let src = "MAX_SIZE = 9999\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let src = "x = 0\ny = 1\nz = 2\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_subscript_index() {
        let src = "x = arr[3]\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_keyword_argument() {
        let src = "connect(timeout=30)\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_default_parameter() {
        let src = "def foo(x=42):\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_float_magic_number() {
        let src = "pi = 3.14159\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3.14159"));
    }
}
