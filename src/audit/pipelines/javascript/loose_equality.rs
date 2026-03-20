use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

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

impl Pipeline for LooseEqualityPipeline {
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
                        let (pattern, suggestion) = match op {
                            "==" => ("loose_equality", "==="),
                            "!=" => ("loose_inequality", "!=="),
                            _ => continue,
                        };

                        let start = node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: pattern.to_string(),
                            message: format!(
                                "`{op}` performs type coercion — use `{suggestion}` for strict comparison"
                            ),
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
}
