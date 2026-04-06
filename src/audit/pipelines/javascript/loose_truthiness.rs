use anyhow::Result;
use tree_sitter::{Node, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{extract_snippet, node_text};

pub struct LooseTruthinessPipeline;

impl LooseTruthinessPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Unwrap a parenthesized_expression to get its inner expression.
    fn unwrap_parens<'a>(node: Node<'a>) -> Option<Node<'a>> {
        if node.kind() == "parenthesized_expression" && node.named_child_count() == 1 {
            node.named_child(0)
        } else {
            Some(node)
        }
    }

    /// Check if a node is a member_expression with property "length".
    fn is_length_member_expression(node: Node, source: &[u8]) -> bool {
        node.kind() == "member_expression"
            && node
                .child_by_field_name("property")
                .is_some_and(|prop| node_text(prop, source) == "length")
    }

    /// Check a condition node (possibly wrapped in parens) for bare `.length`.
    fn check_condition(node: Node, source: &[u8]) -> bool {
        if let Some(inner) = Self::unwrap_parens(node) {
            Self::is_length_member_expression(inner, source)
        } else {
            false
        }
    }

    fn walk_and_check(
        &self,
        node: Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let kind = node.kind();

        let condition_node = match kind {
            "if_statement" | "while_statement" => node.child_by_field_name("condition"),
            "do_statement" => node.child_by_field_name("condition"),
            "ternary_expression" => {
                // In tree-sitter JS, ternary_expression has fields: condition, consequence, alternative
                node.child_by_field_name("condition")
            }
            _ => None,
        };

        if let Some(cond) = condition_node {
            if Self::check_condition(cond, source)
                && !is_nolint_suppressed(source, node, self.name())
            {
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "loose_length_check".to_string(),
                    message:
                        "implicit truthiness check on `.length` — use explicit comparison like `.length > 0`"
                            .to_string(),
                    snippet: extract_snippet(source, node, 1),
                });
            }
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk_and_check(child, source, file_path, findings);
        }
    }
}

impl NodePipeline for LooseTruthinessPipeline {
    fn name(&self) -> &str {
        "loose_truthiness"
    }

    fn description(&self) -> &str {
        "Detects `if(arr.length)` without explicit comparison — implicit truthiness check"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.walk_and_check(tree.root_node(), source, file_path, &mut findings);
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
        let pipeline = LooseTruthinessPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_loose_length_check() {
        let findings = parse_and_check("if (arr.length) { doSomething(); }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "loose_length_check");
    }

    #[test]
    fn skips_explicit_comparison() {
        let findings = parse_and_check("if (arr.length > 0) { doSomething(); }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_other_properties() {
        let findings = parse_and_check("if (obj.visible) { doSomething(); }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_member_condition() {
        let findings = parse_and_check("if (x) { doSomething(); }");
        assert!(findings.is_empty());
    }

    #[test]
    fn while_loop_length() {
        let findings = parse_and_check("while (arr.length) { arr.pop(); }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "loose_length_check");
    }

    #[test]
    fn ternary_length() {
        let findings = parse_and_check("const x = arr.length ? arr[0] : null;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "loose_length_check");
    }

    #[test]
    fn nolint_suppresses() {
        let findings =
            parse_and_check("// NOLINT(loose_truthiness)\nif (arr.length) {}");
        assert!(findings.is_empty());
    }
}
