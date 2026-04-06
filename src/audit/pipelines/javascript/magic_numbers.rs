use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::{is_nolint_suppressed, COMMON_ALLOWED_NUMBERS, is_test_context_js, is_test_file};

use super::primitives::{compile_numeric_literal_query, find_capture_index};

const EXCLUDED_VALUES: &[f64] = &[
    -1.0, 0.0, 1.0, 2.0, 10.0, 100.0, 1000.0, 256.0, 512.0, 1024.0, 2048.0, 4096.0, 8192.0,
];

/// String representations that are always excluded (checked before numeric parsing).
const EXCLUDED_STR_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", "10", "100", "1000", "256", "512", "1024", "2048", "4096",
    "8192",
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
            && index_child.id() == node.id()
        {
            return true;
        }

        false
    }

    /// Parse a numeric literal text into f64, handling hex, binary, octal, and exponential.
    fn parse_numeric_value(text: &str) -> Option<f64> {
        // Try standard float/int parse first (handles decimal and exponential like 1e6)
        if let Ok(v) = text.parse::<f64>() {
            return Some(v);
        }
        // Handle hex: 0x or 0X
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            return u64::from_str_radix(hex, 16).ok().map(|v| v as f64);
        }
        // Handle binary: 0b or 0B
        if let Some(bin) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
            return u64::from_str_radix(bin, 2).ok().map(|v| v as f64);
        }
        // Handle octal: 0o or 0O
        if let Some(oct) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
            return u64::from_str_radix(oct, 8).ok().map(|v| v as f64);
        }
        None
    }

    /// Check if a number node is part of a unary negation expression.
    /// Returns the effective numeric value (negated if applicable) and the
    /// reporting node (the unary_expression if negated, otherwise the number node itself).
    fn resolve_numeric_node<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> Option<(f64, tree_sitter::Node<'a>)> {
        let text = node.utf8_text(source).unwrap_or("");
        let base_value = Self::parse_numeric_value(text)?;

        // Check if parent is a unary_expression with `-` operator
        if let Some(parent) = node.parent() {
            if parent.kind() == "unary_expression" {
                // Get the operator child
                let mut cursor = parent.walk();
                for child in parent.children(&mut cursor) {
                    if !child.is_named() {
                        let op_text = child.utf8_text(source).unwrap_or("");
                        if op_text == "-" {
                            return Some((-base_value, parent));
                        }
                    }
                }
            }
        }

        Some((base_value, node))
    }

    /// Check if a numeric value is in the excluded list.
    fn is_excluded_value(value: f64, text: &str) -> bool {
        // Fast path: check string representation first
        if EXCLUDED_STR_VALUES.contains(&text) || COMMON_ALLOWED_NUMBERS.contains(&text) {
            return true;
        }
        // Numeric path: check parsed f64 value against EXCLUDED_VALUES
        EXCLUDED_VALUES
            .iter()
            .any(|&excluded| (value - excluded).abs() < f64::EPSILON)
    }
}

impl NodePipeline for JsMagicNumbersPipeline {
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
                let raw_text = num_cap.node.utf8_text(source).unwrap_or("");

                // Resolve the effective numeric value, accounting for unary negation
                let Some((value, report_node)) =
                    Self::resolve_numeric_node(num_cap.node, source)
                else {
                    continue;
                };

                // Build the display text (includes sign if negated)
                let display_text = if report_node.id() != num_cap.node.id() {
                    // Negated: use parent's text
                    report_node.utf8_text(source).unwrap_or(raw_text)
                } else {
                    raw_text
                };

                if Self::is_excluded_value(value, raw_text) {
                    continue;
                }

                // For negated values, also check if COMMON_ALLOWED_NUMBERS contains the display
                if report_node.id() != num_cap.node.id()
                    && COMMON_ALLOWED_NUMBERS.contains(&display_text)
                {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node, source) {
                    continue;
                }

                // Skip numbers inside test contexts (describe/it/test blocks)
                if is_test_context_js(num_cap.node, source) {
                    continue;
                }

                // Check for NOLINT suppression
                if is_nolint_suppressed(source, num_cap.node, self.name()) {
                    continue;
                }

                let start = report_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "magic_number".to_string(),
                    message: format!(
                        "magic number `{display_text}` — consider extracting to a named constant for clarity"
                    ),
                    snippet: display_text.to_string(),
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

    #[test]
    fn negative_one_allowed() {
        // -1 is in EXCLUDED_VALUES, should NOT be flagged
        let findings = parse_and_check("const idx = arr.indexOf(x); if (idx === -1) {}");
        assert!(
            findings.is_empty(),
            "expected -1 to be excluded, but got {} findings: {:?}",
            findings.len(),
            findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hex_literal_flagged() {
        // 0xFF = 255, not in EXCLUDED_VALUES, should be flagged
        let findings = parse_and_check("let mask = 0xFF;");
        assert_eq!(findings.len(), 1, "expected 0xFF (255) to be flagged");
        assert!(findings[0].message.contains("0xFF"));
    }

    #[test]
    fn nolint_suppresses() {
        // NOLINT comment on preceding line should suppress the finding
        let findings = parse_and_check("// NOLINT(magic_numbers)\nlet x = 42;");
        assert!(
            findings.is_empty(),
            "expected NOLINT to suppress finding, but got {} findings",
            findings.len()
        );
    }

    #[test]
    fn hex_excluded_value_not_flagged() {
        // 0x100 = 256, which IS in EXCLUDED_VALUES
        let findings = parse_and_check("let x = 0x100;");
        assert!(
            findings.is_empty(),
            "expected 0x100 (256) to be excluded, but got findings: {:?}",
            findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn exponential_literal_flagged() {
        // 1e6 = 1000000, not in EXCLUDED_VALUES
        let findings = parse_and_check("let big = 1e6;");
        assert_eq!(findings.len(), 1, "expected 1e6 to be flagged");
    }

    #[test]
    fn binary_literal_flagged() {
        // 0b1010 = 10, which IS in EXCLUDED_VALUES
        let findings = parse_and_check("let bits = 0b1010;");
        assert!(
            findings.is_empty(),
            "expected 0b1010 (10) to be excluded"
        );
    }
}
