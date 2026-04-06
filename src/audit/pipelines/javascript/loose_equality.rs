use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{compile_binary_expression_query, extract_snippet, node_text};

pub struct LooseEqualityPipeline {
    binary_query: Arc<Query>,
}

impl LooseEqualityPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            binary_query: compile_binary_expression_query()?,
        })
    }
}

impl NodePipeline for LooseEqualityPipeline {
    fn name(&self) -> &str {
        "loose_equality"
    }

    fn description(&self) -> &str {
        "Detects `==` and `!=` which perform type coercion — prefer `===` and `!==`"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.binary_query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.first() {
                let node = cap.node;
                // The operator is an anonymous child; iterate children to find it
                let mut child_cursor = node.walk();
                for child in node.children(&mut child_cursor) {
                    if !child.is_named() {
                        let op = node_text(child, source);
                        let (base_pattern, suggestion) = match op {
                            "==" => ("loose_equality", "==="),
                            "!=" => ("loose_inequality", "!=="),
                            _ => continue,
                        };

                        // Check operands for special cases
                        let first_named = node.named_child(0);
                        let last_named = node
                            .named_child_count()
                            .checked_sub(1)
                            .and_then(|i| node.named_child(i));

                        // typeof comparison: typeof always returns a string, == is safe
                        let is_typeof = first_named.is_some_and(|first| {
                            if first.kind() == "unary_expression" {
                                let mut uc = first.walk();
                                first.children(&mut uc).any(|uchild| {
                                    !uchild.is_named() && node_text(uchild, source) == "typeof"
                                })
                            } else {
                                false
                            }
                        });
                        if is_typeof {
                            continue;
                        }

                        // Check if either operand is null
                        let is_null_check = first_named
                            .map(|n| n.kind() == "null")
                            .unwrap_or(false)
                            || last_named.map(|n| n.kind() == "null").unwrap_or(false);

                        // NOLINT suppression check
                        if is_nolint_suppressed(source, node, self.name()) {
                            continue;
                        }

                        let (severity, pattern, message) = if is_null_check {
                            (
                                "info",
                                "null_coalescing_equality",
                                "== null is an accepted idiom for null/undefined checking — consider keeping or using ===".to_string(),
                            )
                        } else {
                            (
                                "warning",
                                base_pattern,
                                format!("`{op}` performs type coercion — use `{suggestion}` for strict comparison"),
                            )
                        };

                        let start = node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: severity.to_string(),
                            pipeline: self.name().to_string(),
                            pattern: pattern.to_string(),
                            message,
                            snippet: extract_snippet(source, node, 1),
                        });
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = LooseEqualityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_loose_equality() {
        let findings = parse_and_check("if (x == 1) {}");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "loose_equality");
    }

    #[test]
    fn detects_loose_inequality() {
        let findings = parse_and_check("if (x != 1) {}");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "loose_inequality");
    }

    #[test]
    fn skips_strict_equality() {
        let findings = parse_and_check("if (x === 1) {}");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_strict_inequality() {
        let findings = parse_and_check("if (x !== 1) {}");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_other_operators() {
        let findings = parse_and_check("let z = x + y;");
        assert!(findings.is_empty());
    }

    #[test]
    fn null_check_idiom() {
        let findings = parse_and_check("if (x == null) {}");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
        assert_eq!(findings[0].pattern, "null_coalescing_equality");
    }

    #[test]
    fn typeof_comparison_suppressed() {
        let findings = parse_and_check("if (typeof x == \"string\") {}");
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn regular_loose_equality() {
        let findings = parse_and_check("if (x == 0) {}");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn nolint_suppresses() {
        let findings = parse_and_check("// NOLINT(loose_equality)\nif (x == 1) {}");
        assert_eq!(findings.len(), 0);
    }
}
