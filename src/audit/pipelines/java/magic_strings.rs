use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{has_suppress_warnings, is_test_file};

use super::primitives::{
    compile_method_invocation_query, extract_snippet, find_capture_index, node_text,
};

const WELL_KNOWN_VALUES: &[&str] = &[
    "\"true\"",
    "\"false\"",
    "\"null\"",
    "\"yes\"",
    "\"no\"",
    "\"GET\"",
    "\"POST\"",
    "\"PUT\"",
    "\"DELETE\"",
    "\"PATCH\"",
    "\"HEAD\"",
    "\"OPTIONS\"",
    "\"utf-8\"",
    "\"UTF-8\"",
    "\"application/json\"",
    "\"text/html\"",
];

const TARGET_METHODS: &[&str] = &[
    "equals",
    "equalsIgnoreCase",
    "contains",
    "startsWith",
    "endsWith",
];

pub struct MagicStringsPipeline {
    invocation_query: Arc<Query>,
}

impl MagicStringsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            invocation_query: compile_method_invocation_query()?,
        })
    }
}

/// Walk up from a node to find the enclosing method_declaration or constructor_declaration.
fn find_enclosing_method(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "method_declaration" || p.kind() == "constructor_declaration" {
            return Some(p);
        }
        parent = p.parent();
    }
    None
}

/// Check if a string literal is trivial (empty, single-char, or well-known).
fn is_trivial_string(text: &str) -> bool {
    // Skip empty strings "" (len 2) and single-char strings like "," (len 3)
    if text.len() <= 3 {
        return true;
    }
    WELL_KNOWN_VALUES.contains(&text)
}

/// Recursively collect all nodes of a given kind from a subtree.
fn collect_nodes_of_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
    out: &mut Vec<tree_sitter::Node<'a>>,
) {
    if node.kind() == kind {
        out.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_nodes_of_kind(child, kind, out);
    }
}

impl GraphPipeline for MagicStringsPipeline {
    fn name(&self) -> &str {
        "magic_strings"
    }

    fn description(&self) -> &str {
        "Detects string comparison and matching calls with magic string literals — extract to constants or enums"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let method_name_idx = find_capture_index(&self.invocation_query, "method_name");
        let args_idx = find_capture_index(&self.invocation_query, "args");
        let invocation_idx = find_capture_index(&self.invocation_query, "invocation");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let invocation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == invocation_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(args_node), Some(invocation_node)) =
                (name_node, args_node, invocation_node)
            {
                let method_name = node_text(name_node, source);
                if !TARGET_METHODS.contains(&method_name) {
                    continue;
                }

                // Check @SuppressWarnings on enclosing method
                if let Some(enclosing) = find_enclosing_method(invocation_node)
                    && has_suppress_warnings(enclosing, source, "magic-string")
                {
                    continue;
                }

                // Check if the object (receiver) of the method invocation is a string_literal
                // e.g. "ADMIN".equals(role)
                let object_node = invocation_node.child_by_field_name("object");
                if let Some(obj) = object_node
                    && obj.kind() == "string_literal"
                {
                    let text = node_text(obj, source);
                    if !is_trivial_string(text) {
                        let start = invocation_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "magic_string".to_string(),
                            message: format!(
                                "{text}.{method_name}() uses a magic string — extract to a constant or enum"
                            ),
                            snippet: extract_snippet(source, invocation_node, 3),
                        });
                        // Already reported for this invocation, skip argument check
                        continue;
                    }
                }

                // Check if arguments contain a non-trivial string literal
                let magic_string_node = (0..args_node.named_child_count())
                    .filter_map(|i| args_node.named_child(i))
                    .find(|child| {
                        child.kind() == "string_literal" && !is_trivial_string(node_text(*child, source))
                    });

                if let Some(string_node) = magic_string_node {
                    let string_text = node_text(string_node, source);
                    let start = invocation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "magic_string".to_string(),
                        message: format!(
                            ".{method_name}({string_text}) uses a magic string — extract to a constant or enum"
                        ),
                        snippet: extract_snippet(source, invocation_node, 3),
                    });
                }
            }
        }

        // ---- Pass 2: detect string literals in switch case labels ----
        let mut switch_label_nodes = Vec::new();
        collect_nodes_of_kind(tree.root_node(), "switch_label", &mut switch_label_nodes);

        for label_node in switch_label_nodes {
            let mut string_literals = Vec::new();
            collect_nodes_of_kind(label_node, "string_literal", &mut string_literals);

            for string_node in string_literals {
                let text = node_text(string_node, source);
                if is_trivial_string(text) {
                    continue;
                }

                // Check @SuppressWarnings on enclosing method
                if let Some(enclosing) = find_enclosing_method(string_node)
                    && has_suppress_warnings(enclosing, source, "magic-string")
                {
                    continue;
                }

                let start = string_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "magic_string".to_string(),
                    message: format!(
                        "switch case {text} uses a magic string — extract to a constant or enum"
                    ),
                    snippet: extract_snippet(source, string_node, 3),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MagicStringsPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Foo.java",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_equals_magic_string() {
        let src = "class Foo { void m() { role.equals(\"ADMIN\"); } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_string");
        assert!(findings[0].message.contains("\"ADMIN\""));
    }

    #[test]
    fn detects_equals_ignore_case_magic_string() {
        let src = "class Foo { void m() { s.equalsIgnoreCase(\"admin\"); } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_equals_with_variable() {
        let src = "class Foo { void m() { a.equals(b); } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_equals_empty_string() {
        let src = "class Foo { void m() { s.equals(\"\"); } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_reversed_equals() {
        let src = r#"class Foo { void m(String role) { "ADMIN".equals(role); } }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("\"ADMIN\""));
    }

    #[test]
    fn test_contains_magic_string() {
        let src = r#"class Foo { void m(String s) { s.contains("SECRET_PREFIX"); } }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn test_single_char_skipped() {
        let src = r#"class Foo { void m(String s) { s.equals(","); } }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_well_known_values() {
        let src = r#"class Foo { void m(String s) { s.equals("true"); } }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_test_file_skipped() {
        let src = r#"class FooTest { void m() { s.equals("ADMIN"); } }"#;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let pipeline = MagicStringsPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: src.as_bytes(),
            file_path: "src/test/java/FooTest.java",
            id_counts: &id_counts,
            graph: &graph,
        };
        let findings = pipeline.check(&ctx);
        assert!(findings.is_empty());
    }
}
