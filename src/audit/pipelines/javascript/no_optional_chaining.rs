use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{compile_member_expression_query, extract_snippet, find_capture_index};

const DEPTH_THRESHOLD: usize = 4;

/// Well-known global/built-in roots that are always defined and never null.
const SAFE_ROOTS: &[&str] = &[
    "document",
    "window",
    "navigator",
    "location",
    "history",
    "screen",
    "process",
    "module",
    "require",
    "global",
    "globalThis",
    "Math",
    "JSON",
    "Object",
    "Array",
    "console",
    "Number",
    "String",
    "Date",
    "RegExp",
    "Promise",
    "Reflect",
    "Proxy",
    "Intl",
    "this",
    "self",
];

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

    /// Walk down the `object` field of the member_expression chain until we reach
    /// a non-member_expression node. If it is an `identifier` or `this`, return
    /// the text as the root name.
    fn root_identifier<'a>(node: tree_sitter::Node<'a>, source: &'a [u8]) -> Option<&'a str> {
        let mut current = node;
        loop {
            if let Some(obj) = current.child_by_field_name("object") {
                if obj.kind() == "member_expression" {
                    current = obj;
                    continue;
                }
                // Reached a non-member_expression node
                if obj.kind() == "identifier" || obj.kind() == "this" {
                    return obj.utf8_text(source).ok();
                }
                return None;
            }
            // No object field — current node itself might be the root
            if current.kind() == "identifier" || current.kind() == "this" {
                return current.utf8_text(source).ok();
            }
            return None;
        }
    }

    /// Determine severity from chain depth.
    fn severity_for_depth(depth: usize) -> &'static str {
        match depth {
            0..=5 => "info",
            6..=7 => "warning",
            _ => "error",
        }
    }
}

impl NodePipeline for NoOptionalChainingPipeline {
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

                // Suppress findings rooted on well-known safe globals
                if let Some(root) = Self::root_identifier(node, source) {
                    if SAFE_ROOTS.contains(&root) {
                        continue;
                    }
                }

                // Suppress if NOLINT comment present
                if is_nolint_suppressed(source, node, self.name()) {
                    continue;
                }

                let severity = Self::severity_for_depth(depth);
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
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

    #[test]
    fn safe_root_document() {
        let findings = parse_and_check("let x = document.body.style.display;");
        assert!(findings.is_empty(), "document is a safe root, should be suppressed");
    }

    #[test]
    fn safe_root_process() {
        let findings = parse_and_check("let x = process.env.NODE_ENV.toLowerCase();");
        assert!(findings.is_empty(), "process is a safe root, should be suppressed");
    }

    #[test]
    fn depth_6_warning() {
        let findings = parse_and_check("let x = a.b.c.d.e.f;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn nolint_suppresses() {
        let findings =
            parse_and_check("// NOLINT(no_optional_chaining)\nlet x = a.b.c.d;");
        assert!(findings.is_empty(), "NOLINT comment should suppress the finding");
    }
}
