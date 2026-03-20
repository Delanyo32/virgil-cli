use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_member_expression_query, extract_snippet, find_capture_index};

const DEPTH_THRESHOLD: usize = 4;

pub struct NoOptionalChainingPipeline {
    member_query: Arc<Query>,
}

impl NoOptionalChainingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            member_query: compile_member_expression_query()?,
        })
    }

    /// Count the number of segments in a member_expression chain.
    /// `a.b.c.d` = 4 segments (the root identifier + 3 property accesses).
    fn chain_depth(node: tree_sitter::Node) -> usize {
        let mut segments = 1; // count current property access
        let mut current = node;
        while let Some(obj) = current.child_by_field_name("object") {
            segments += 1;
            if obj.kind() == "member_expression" {
                current = obj;
            } else {
                break;
            }
        }
        segments
    }

    /// Check if the chain (or any ancestor) uses optional chaining.
    fn has_optional_chaining(node: tree_sitter::Node) -> bool {
        // Check if this node or any child member_expression is an optional_chain_expression
        let mut current = node;
        loop {
            if current.kind() == "optional_chain_expression" {
                return true;
            }
            // Check parent — optional_chain_expression wraps member_expression
            if let Some(parent) = current.parent()
                && parent.kind() == "optional_chain_expression"
            {
                return true;
            }
            if let Some(obj) = current.child_by_field_name("object")
                && (obj.kind() == "member_expression" || obj.kind() == "optional_chain_expression")
            {
                current = obj;
                continue;
            }
            break;
        }
        false
    }
}

impl Pipeline for NoOptionalChainingPipeline {
    fn name(&self) -> &str {
        "no_optional_chaining"
    }

    fn description(&self) -> &str {
        "Detects deep property chains (4+ levels) without optional chaining (?.)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.member_query, tree.root_node(), source);

        let member_idx = find_capture_index(&self.member_query, "member");

        while let Some(m) = matches.next() {
            let member_cap = m.captures.iter().find(|c| c.index as usize == member_idx);

            if let Some(cap) = member_cap {
                let node = cap.node;

                // Only flag outermost expression (skip if parent is also member_expression)
                if let Some(parent) = node.parent()
                    && parent.kind() == "member_expression"
                {
                    continue;
                }

                let depth = Self::chain_depth(node);
                if depth < DEPTH_THRESHOLD {
                    continue;
                }

                if Self::has_optional_chaining(node) {
                    continue;
                }

                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "deep_property_chain".to_string(),
                    message: format!(
                        "property chain depth {depth} without optional chaining — consider using `?.`"
                    ),
                    snippet: extract_snippet(source, node, 1),
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
        let pipeline = NoOptionalChainingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_deep_chain() {
        let findings = parse_and_check("let x = a.b.c.d;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "deep_property_chain");
    }

    #[test]
    fn skips_shallow_chain() {
        let findings = parse_and_check("let x = a.b.c;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_single_member() {
        let findings = parse_and_check("let x = a.b;");
        assert!(findings.is_empty());
    }
}
