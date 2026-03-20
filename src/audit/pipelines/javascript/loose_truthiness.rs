use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_if_statement_query, extract_snippet, find_capture_index, node_text,
};

pub struct LooseTruthinessPipeline {
    if_query: Arc<Query>,
}

impl LooseTruthinessPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            if_query: compile_if_statement_query()?,
        })
    }
}

impl Pipeline for LooseTruthinessPipeline {
    fn name(&self) -> &str {
        "loose_truthiness"
    }

    fn description(&self) -> &str {
        "Detects `if(arr.length)` without explicit comparison — implicit truthiness check"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.if_query, tree.root_node(), source);

        let condition_idx = find_capture_index(&self.if_query, "condition");
        let if_idx = find_capture_index(&self.if_query, "if_stmt");

        while let Some(m) = matches.next() {
            let cond_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == condition_idx);
            let if_cap = m.captures.iter().find(|c| c.index as usize == if_idx);

            if let (Some(cond), Some(if_node)) = (cond_cap, if_cap) {
                // The condition is wrapped in parenthesized_expression; get inner expression
                let inner = if cond.node.named_child_count() == 1 {
                    cond.node.named_child(0)
                } else {
                    None
                };

                if let Some(inner) = inner {
                    // Check if it's a member_expression with property "length"
                    if inner.kind() == "member_expression"
                        && let Some(prop) = inner.child_by_field_name("property")
                        && node_text(prop, source) == "length"
                    {
                        let start = if_node.node.start_position();
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
                                    snippet: extract_snippet(source, if_node.node, 1),
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
}
