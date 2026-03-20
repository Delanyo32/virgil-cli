use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_if_statement_query, extract_snippet, find_capture_index};

const CHAIN_THRESHOLD: usize = 3;

pub struct InstanceofChainsPipeline {
    if_query: Arc<Query>,
}

impl InstanceofChainsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            if_query: compile_if_statement_query()?,
        })
    }
}

impl Pipeline for InstanceofChainsPipeline {
    fn name(&self) -> &str {
        "instanceof_chains"
    }

    fn description(&self) -> &str {
        "Detects long if/else-if chains using instanceof — consider using polymorphism or pattern matching"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.if_query, tree.root_node(), source);

        let condition_idx = find_capture_index(&self.if_query, "condition");
        let if_stmt_idx = find_capture_index(&self.if_query, "if_stmt");

        // Track already-reported if-statement IDs to avoid duplicates
        let mut reported: HashSet<usize> = HashSet::new();

        while let Some(m) = matches.next() {
            let if_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == if_stmt_idx)
                .map(|c| c.node);
            let condition_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == condition_idx)
                .map(|c| c.node);

            if let (Some(if_node), Some(condition_node)) = (if_node, condition_node) {
                // Only process the outermost if in a chain (skip if already part of reported chain)
                if reported.contains(&if_node.id()) {
                    continue;
                }

                // Check if this if's condition contains instanceof
                if !contains_instanceof(condition_node) {
                    continue;
                }

                // Count the chain of instanceof checks
                let (count, chain_nodes) = count_instanceof_chain(if_node, source);

                if count >= CHAIN_THRESHOLD {
                    // Mark all nodes in the chain as reported
                    for id in &chain_nodes {
                        reported.insert(*id);
                    }

                    let start = if_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "instanceof_chain".to_string(),
                        message: format!(
                            "if/else-if chain has {count} instanceof checks — consider using polymorphism or pattern matching"
                        ),
                        snippet: extract_snippet(source, if_node, 5),
                    });
                }
            }
        }

        findings
    }
}

fn contains_instanceof(node: tree_sitter::Node) -> bool {
    if node.kind() == "instanceof_expression" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_instanceof(child) {
            return true;
        }
    }
    false
}

fn count_instanceof_chain(if_node: tree_sitter::Node, _source: &[u8]) -> (usize, Vec<usize>) {
    let mut count = 0;
    let mut node_ids = Vec::new();
    let mut current = Some(if_node);

    while let Some(node) = current {
        if node.kind() != "if_statement" {
            break;
        }

        node_ids.push(node.id());

        // Check condition for instanceof
        if let Some(condition) = node.child_by_field_name("condition")
            && contains_instanceof(condition)
        {
            count += 1;
        }

        // Follow the else-if chain via "alternative" field
        current = node.child_by_field_name("alternative").and_then(|alt| {
            if alt.kind() == "if_statement" {
                Some(alt)
            } else {
                // Check if the block contains an if_statement (else { if (...) })
                None
            }
        });
    }

    (count, node_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = InstanceofChainsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_3_chain() {
        let src = r#"
class Foo {
    void m(Object o) {
        if (o instanceof String) {
        } else if (o instanceof Integer) {
        } else if (o instanceof Double) {
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "instanceof_chain");
        assert!(findings[0].message.contains("3 instanceof"));
    }

    #[test]
    fn detects_5_chain() {
        let src = r#"
class Foo {
    void m(Object o) {
        if (o instanceof String) {
        } else if (o instanceof Integer) {
        } else if (o instanceof Double) {
        } else if (o instanceof Float) {
        } else if (o instanceof Long) {
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("5 instanceof"));
    }

    #[test]
    fn clean_2_chain() {
        let src = r#"
class Foo {
    void m(Object o) {
        if (o instanceof String) {
        } else if (o instanceof Integer) {
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_separate_ifs() {
        let src = r#"
class Foo {
    void m(Object a, Object b, Object c) {
        if (a instanceof String) { }
        if (b instanceof Integer) { }
        if (c instanceof Double) { }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
