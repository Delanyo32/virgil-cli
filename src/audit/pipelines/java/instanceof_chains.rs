use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::has_suppress_warnings;

use super::primitives::{compile_if_statement_query, extract_snippet, find_capture_index, node_text};

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

impl GraphPipeline for InstanceofChainsPipeline {
    fn name(&self) -> &str {
        "instanceof_chains"
    }

    fn description(&self) -> &str {
        "Detects long if/else-if chains using instanceof — consider using polymorphism or pattern matching"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

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

                // Check @SuppressWarnings on enclosing method/class
                if has_suppress_warnings(if_node, source, "instanceof-chain") {
                    continue;
                }

                // Count the chain of instanceof checks and collect types
                let (count, chain_nodes, types, uses_pattern_matching) =
                    count_instanceof_chain(if_node, source);

                if count >= CHAIN_THRESHOLD {
                    // Mark all nodes in the chain as reported
                    for id in &chain_nodes {
                        reported.insert(*id);
                    }

                    let severity = if count >= 8 {
                        "error"
                    } else if count >= 5 {
                        "warning"
                    } else {
                        "info"
                    };

                    let types_str = types.join(", ");
                    let pattern_note = if uses_pattern_matching {
                        " (uses pattern matching instanceof)"
                    } else {
                        ""
                    };

                    let start = if_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "instanceof_chain".to_string(),
                        message: format!(
                            "if/else-if chain has {count} instanceof checks ({types_str}) — consider using polymorphism or pattern matching{pattern_note}"
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

/// Extract types from instanceof expressions within a condition node.
/// Also detects pattern matching instanceof (Java 16+) where a binding name follows the type.
fn extract_instanceof_types(
    condition_node: tree_sitter::Node,
    source: &[u8],
    types: &mut Vec<String>,
    uses_pattern_matching: &mut bool,
) {
    if condition_node.kind() == "instanceof_expression" {
        // In Java tree-sitter, instanceof_expression has: left, "instanceof", type
        // Pattern matching adds a binding name after the type
        if let Some(type_node) = condition_node.child_by_field_name("type") {
            types.push(node_text(type_node, source).to_string());
        } else if condition_node.named_child_count() >= 2
            && let Some(last) = condition_node.named_child(condition_node.named_child_count() - 1)
        {
            types.push(node_text(last, source).to_string());
        }
        // Detect pattern matching: if there are 3+ named children (object, type, binding name)
        if condition_node.named_child_count() >= 3 {
            *uses_pattern_matching = true;
        }
    }
    let mut cursor = condition_node.walk();
    for child in condition_node.children(&mut cursor) {
        extract_instanceof_types(child, source, types, uses_pattern_matching);
    }
}

/// Count instanceof checks in an if/else-if chain, collecting node IDs, types, and
/// whether pattern matching instanceof is used.
fn count_instanceof_chain(
    if_node: tree_sitter::Node,
    source: &[u8],
) -> (usize, Vec<usize>, Vec<String>, bool) {
    let mut count = 0;
    let mut node_ids = Vec::new();
    let mut types = Vec::new();
    let mut uses_pattern_matching = false;
    let mut current = Some(if_node);

    while let Some(node) = current {
        if node.kind() != "if_statement" {
            break;
        }

        node_ids.push(node.id());

        // Check condition for instanceof and extract types
        if let Some(condition) = node.child_by_field_name("condition")
            && contains_instanceof(condition)
        {
            count += 1;
            extract_instanceof_types(condition, source, &mut types, &mut uses_pattern_matching);
        }

        // Follow the else-if chain via "alternative" field
        current = node.child_by_field_name("alternative").and_then(|alt| {
            if alt.kind() == "if_statement" {
                Some(alt)
            } else if alt.kind() == "block" {
                // else { if (...) { } } pattern — check if block has single if_statement child
                let named_count = alt.named_child_count();
                if named_count == 1 {
                    alt.named_child(0).filter(|c| c.kind() == "if_statement")
                } else {
                    None
                }
            } else {
                None
            }
        });
    }

    (count, node_ids, types, uses_pattern_matching)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = InstanceofChainsPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Test.java",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn test_else_block_if_pattern() {
        let src = r#"
class Foo {
    void m(Object o) {
        if (o instanceof String) {
        } else {
            if (o instanceof Integer) {
            } else {
                if (o instanceof Double) {
                }
            }
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3 instanceof"));
    }

    #[test]
    fn test_severity_graduation() {
        // Build 8 instanceof chain -> "error"
        let src = r#"
class Foo {
    void m(Object o) {
        if (o instanceof String) {
        } else if (o instanceof Integer) {
        } else if (o instanceof Double) {
        } else if (o instanceof Float) {
        } else if (o instanceof Long) {
        } else if (o instanceof Short) {
        } else if (o instanceof Byte) {
        } else if (o instanceof Character) {
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn test_severity_info_for_3() {
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
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn test_severity_warning_for_5() {
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
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn test_types_reported() {
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
        assert!(findings[0].message.contains("String"));
        assert!(findings[0].message.contains("Integer"));
        assert!(findings[0].message.contains("Double"));
    }

    #[test]
    fn test_suppress_warnings() {
        let src = r#"
class Foo {
    @SuppressWarnings("instanceof-chain")
    void m(Object o) {
        if (o instanceof String) {
        } else if (o instanceof Integer) {
        } else if (o instanceof Double) {
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
