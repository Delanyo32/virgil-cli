use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_member_access_query, compile_return_null_query, extract_snippet, find_capture_index,
    has_modifier, is_csharp_suppressed, node_text,
};

/// LINQ/fluent method names that are safe to chain (return non-null).
const FLUENT_METHOD_NAMES: &[&str] = &[
    "Where", "Select", "OrderBy", "OrderByDescending", "ThenBy", "GroupBy", "ToList", "ToArray",
    "First", "FirstOrDefault", "Single", "SingleOrDefault", "Any", "All", "Count", "Sum", "Max",
    "Min", "Average", "Distinct", "Take", "Skip", "Concat", "Union", "Intersect", "Except",
    "Append", "Prepend", "Reverse", "Zip",
    "WithName", "WithValue", "AddXxx", "Build", "ToString", "Trim", "ToLower", "ToUpper",
    "Replace", "Substring", "Split", "Join",
];

pub struct NullReferenceRiskPipeline {
    return_null_query: Arc<Query>,
    member_access_query: Arc<Query>,
}

impl NullReferenceRiskPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            return_null_query: compile_return_null_query()?,
            member_access_query: compile_member_access_query()?,
        })
    }
}

impl GraphPipeline for NullReferenceRiskPipeline {
    fn name(&self) -> &str {
        "null_reference_risk"
    }

    fn description(&self) -> &str {
        "Detects explicit null returns and deep member access chains without null-conditional operators"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();

        // Pattern 1: explicit null returns
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.return_null_query, tree.root_node(), source);
            let return_stmt_idx = find_capture_index(&self.return_null_query, "return_stmt");

            while let Some(m) = matches.next() {
                let return_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == return_stmt_idx)
                    .map(|c| c.node);
                if let Some(return_node) = return_node {
                    if is_csharp_suppressed(source, return_node, "null_reference_risk") {
                        continue;
                    }

                    // Skip if method has nullable return type (T?)
                    if is_in_nullable_return_method(return_node, source) {
                        continue;
                    }

                    // Severity: public method → warning, private → info
                    let severity = if is_in_public_method(return_node, source) {
                        "warning"
                    } else {
                        "info"
                    };

                    let start = return_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "explicit_null_return".to_string(),
                        message: "explicit `return null` \u{2014} consider returning a default, throwing, or using nullable reference types".to_string(),
                        snippet: extract_snippet(source, return_node, 3),
                    });
                }
            }
        }

        // Pattern 2: deep member access chains (nested member_access_expression without ?.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.member_access_query, tree.root_node(), source);
            let member_access_idx = find_capture_index(&self.member_access_query, "member_access");

            while let Some(m) = matches.next() {
                let access_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == member_access_idx)
                    .map(|c| c.node);
                if let Some(access_node) = access_node {
                    let depth = count_member_access_depth(access_node);
                    if depth >= 3 {
                        if !has_conditional_access_ancestor(access_node) {
                            if access_node
                                .parent()
                                .is_none_or(|p| p.kind() != "member_access_expression")
                            {
                                // Skip fluent/LINQ chains
                                if is_fluent_chain(access_node, source) {
                                    continue;
                                }

                                if is_csharp_suppressed(source, access_node, "null_reference_risk")
                                {
                                    continue;
                                }

                                // Severity: depth 3 → info, 4+ → warning
                                let severity = if depth >= 4 { "warning" } else { "info" };

                                let start = access_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: severity.to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "deep_member_chain".to_string(),
                                    message: format!(
                                        "deep member access chain (depth {depth}) without null-conditional operator (?.) \u{2014} risk of NullReferenceException"
                                    ),
                                    snippet: extract_snippet(source, access_node, 3),
                                });
                            }
                        }
                    }
                }
            }
        }

        findings
    }
}

fn count_member_access_depth(node: tree_sitter::Node) -> usize {
    let mut depth = 1;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "member_access_expression" {
            depth = depth.max(1 + count_member_access_depth(child));
        }
    }
    depth
}

fn has_conditional_access_ancestor(node: tree_sitter::Node) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "conditional_access_expression" {
            return true;
        }
        current = n.parent();
    }
    false
}

/// Check if a chain looks like a fluent API / LINQ call chain (method invocations, not plain field access).
fn is_fluent_chain(node: tree_sitter::Node, source: &[u8]) -> bool {
    // If the parent is an invocation_expression, this is a method call chain
    if let Some(parent) = node.parent() {
        if parent.kind() == "invocation_expression" {
            // Check if the member name is a known fluent method
            if let Some(name) = node.child_by_field_name("name") {
                let name_text = node_text(name, source);
                if FLUENT_METHOD_NAMES.contains(&name_text) {
                    return true;
                }
            }
        }
    }
    // Check child chain too
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "member_access_expression" && is_fluent_chain(child, source) {
            return true;
        }
    }
    false
}

/// Check if the return statement is inside a method with a nullable return type (T?).
fn is_in_nullable_return_method(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "method_declaration" {
            if let Some(ret_type) = n.child_by_field_name("returns") {
                let ret_text = ret_type.utf8_text(source).unwrap_or("");
                if ret_text.ends_with('?') || ret_type.kind() == "nullable_type" {
                    return true;
                }
            }
            return false;
        }
        current = n.parent();
    }
    false
}

fn is_in_public_method(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "method_declaration" {
            return has_modifier(n, source, "public");
        }
        current = n.parent();
    }
    false
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NullReferenceRiskPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Service.cs",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_return_null() {
        let src = r#"
class Foo {
    public object Bar() {
        return null;
    }
}
"#;
        let findings = parse_and_check(src);
        let null_returns: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "explicit_null_return")
            .collect();
        assert_eq!(null_returns.len(), 1);
        assert_eq!(null_returns[0].severity, "warning");
    }

    #[test]
    fn detects_deep_chain() {
        let src = r#"
class Foo {
    void Bar() {
        var x = a.b.c.d;
    }
}
"#;
        let findings = parse_and_check(src);
        let chains: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "deep_member_chain")
            .collect();
        assert_eq!(chains.len(), 1);
    }

    #[test]
    fn clean_short_chain() {
        let src = r#"
class Foo {
    void Bar() {
        var x = a.b;
    }
}
"#;
        let findings = parse_and_check(src);
        let chains: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "deep_member_chain")
            .collect();
        assert!(chains.is_empty());
    }

    #[test]
    fn clean_no_null_return() {
        let src = r#"
class Foo {
    int Bar() {
        return 42;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nullable_return_excluded() {
        let src = r#"
class Foo {
    string? GetName() {
        return null;
    }
}
"#;
        let findings = parse_and_check(src);
        let null_returns: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "explicit_null_return")
            .collect();
        assert!(null_returns.is_empty());
    }

    #[test]
    fn private_method_is_info_severity() {
        let src = r#"
class Foo {
    private object Bar() {
        return null;
    }
}
"#;
        let findings = parse_and_check(src);
        let null_returns: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "explicit_null_return")
            .collect();
        assert_eq!(null_returns.len(), 1);
        assert_eq!(null_returns[0].severity, "info");
    }

    #[test]
    fn suppressed_by_nolint() {
        let src = r#"
class Foo {
    public object Bar() {
        // NOLINT
        return null;
    }
}
"#;
        let findings = parse_and_check(src);
        let null_returns: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "explicit_null_return")
            .collect();
        assert!(null_returns.is_empty());
    }
}
