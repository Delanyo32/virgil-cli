use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_return_null_query, extract_snippet, find_capture_index, has_modifier, node_text,
};
use crate::audit::pipelines::helpers::{has_annotation, has_suppress_warnings, is_test_file};

/// Primitive types that cannot be null in Java — skip findings for these return types.
const PRIMITIVE_TYPES: &[&str] = &[
    "void", "int", "long", "double", "float", "boolean", "byte", "short", "char",
];

pub struct NullReturnsPipeline {
    return_null_query: Arc<Query>,
}

impl NullReturnsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            return_null_query: compile_return_null_query()?,
        })
    }
}

/// Check if a node is inside a `catch_clause`. Stops at method/class boundaries.
fn is_inside_catch(node: tree_sitter::Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "catch_clause" {
            return true;
        }
        if p.kind() == "method_declaration" || p.kind() == "class_declaration" {
            return false;
        }
        parent = p.parent();
    }
    false
}

/// Check if a `return_statement` contains a ternary expression with a `null_literal`.
fn return_has_ternary_null(return_node: tree_sitter::Node) -> bool {
    for i in 0..return_node.named_child_count() {
        if let Some(child) = return_node.named_child(i)
            && child.kind() == "ternary_expression"
        {
            return contains_null_literal(child);
        }
    }
    false
}

/// Recursively check if any descendant is a `null_literal`.
fn contains_null_literal(node: tree_sitter::Node) -> bool {
    if node.kind() == "null_literal" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_null_literal(child) {
            return true;
        }
    }
    false
}

/// Walk the tree collecting all `return_statement` nodes.
fn collect_return_statements<'a>(node: tree_sitter::Node<'a>, out: &mut Vec<tree_sitter::Node<'a>>) {
    if node.kind() == "return_statement" {
        out.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_return_statements(child, out);
    }
}

/// Extract the return type text from a method_declaration node.
fn method_return_type_text<'a>(method_node: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
    method_node
        .child_by_field_name("type")
        .map(|t| node_text(t, source))
        .unwrap_or("")
}

/// Find the enclosing method_declaration, returning (method_name, method_node).
/// Returns None if inside a constructor or no method found.
fn find_enclosing_method<'a>(
    return_node: tree_sitter::Node<'a>,
    source: &'a [u8],
) -> Option<(String, tree_sitter::Node<'a>)> {
    let mut parent = return_node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "method_declaration" => {
                let name = p
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source).to_string());
                if let Some(name) = name {
                    return Some((name, p));
                }
                return None;
            }
            "constructor_declaration" => {
                // Skip constructors
                return None;
            }
            _ => {
                parent = p.parent();
            }
        }
    }
    None
}

/// Determine if a finding should be skipped based on annotations and return type.
/// Returns None if skipped, or Some((pattern, severity, message)) if a finding should be emitted.
fn classify_null_return(
    method_name: &str,
    method_node: tree_sitter::Node,
    return_node: tree_sitter::Node,
    source: &[u8],
) -> Option<(String, String, String)> {
    // Skip @Test / @Deprecated
    if has_annotation(method_node, source, "Test")
        || has_annotation(method_node, source, "Deprecated")
    {
        return None;
    }

    // Skip @Nullable / @CheckForNull / @SuppressWarnings("null")
    if has_annotation(method_node, source, "Nullable")
        || has_annotation(method_node, source, "CheckForNull")
        || has_suppress_warnings(method_node, source, "null")
    {
        return None;
    }

    // Skip null returns inside catch blocks (defer to exception_swallowing pipeline)
    if is_inside_catch(return_node) {
        return None;
    }

    // Get return type text
    let return_type = method_return_type_text(method_node, source);

    // Skip void and primitive return types (null can't be returned from these)
    if PRIMITIVE_TYPES.contains(&return_type) {
        return None;
    }

    // Check if return type is Optional — returning null from Optional is a bug
    if return_type.starts_with("Optional") {
        return Some((
            "null_return_optional".to_string(),
            "error".to_string(),
            format!(
                "method `{method_name}` has return type `Optional` but returns null — this is a bug, use `Optional.empty()` instead"
            ),
        ));
    }

    // Public vs private severity
    let is_public = has_modifier(method_node, source, "public");
    let severity = if is_public { "warning" } else { "info" };

    Some((
        "null_return".to_string(),
        severity.to_string(),
        format!("method `{method_name}` returns null — consider returning Optional<T>"),
    ))
}

impl GraphPipeline for NullReturnsPipeline {
    fn name(&self) -> &str {
        "null_returns"
    }

    fn description(&self) -> &str {
        "Detects methods that return null — consider returning Optional<T> instead"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        // Skip test files entirely
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut reported_lines: HashSet<u32> = HashSet::new();

        // ---- Pass 1: tree-sitter query for `return null;` ----
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.return_null_query, tree.root_node(), source);

        let return_stmt_idx = find_capture_index(&self.return_null_query, "return_stmt");

        while let Some(m) = matches.next() {
            let return_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == return_stmt_idx)
                .map(|c| c.node);

            let Some(return_node) = return_node else {
                continue;
            };

            let Some((method_name, method_node)) = find_enclosing_method(return_node, source)
            else {
                continue;
            };

            let Some((pattern, severity, message)) =
                classify_null_return(&method_name, method_node, return_node, source)
            else {
                continue;
            };

            let start = return_node.start_position();
            let line = start.row as u32 + 1;
            reported_lines.insert(line);

            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line,
                column: start.column as u32 + 1,
                severity,
                pipeline: self.name().to_string(),
                pattern,
                message,
                snippet: extract_snippet(source, return_node, 3),
            });
        }

        // ---- Pass 2: detect ternary null in return statements ----
        let mut return_nodes = Vec::new();
        collect_return_statements(tree.root_node(), &mut return_nodes);

        for return_node in return_nodes {
            let start = return_node.start_position();
            let line = start.row as u32 + 1;

            // Skip if already reported by pass 1
            if reported_lines.contains(&line) {
                continue;
            }

            if !return_has_ternary_null(return_node) {
                continue;
            }

            let Some((method_name, method_node)) = find_enclosing_method(return_node, source)
            else {
                continue;
            };

            let Some((pattern, severity, message)) =
                classify_null_return(&method_name, method_node, return_node, source)
            else {
                continue;
            };

            reported_lines.insert(line);

            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line,
                column: start.column as u32 + 1,
                severity,
                pipeline: self.name().to_string(),
                pattern,
                message,
                snippet: extract_snippet(source, return_node, 3),
            });
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
        let pipeline = NullReturnsPipeline::new().unwrap();
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
    fn detects_null_return() {
        let src = "class Foo { public String findUser() { return null; } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "null_return");
        assert!(findings[0].message.contains("findUser"));
    }

    #[test]
    fn skips_constructor() {
        let src = "class Foo { Foo() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_non_null_return() {
        let src = "class Foo { String getName() { return \"hello\"; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_nullable_annotation_skipped() {
        let src = "class Foo { @Nullable String find() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_check_for_null_annotation_skipped() {
        let src = "class Foo { @CheckForNull String find() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_public_vs_private_severity() {
        let src = r#"
class Foo {
    public String findPublic() { return null; }
    private String findPrivate() { return null; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        let pub_f = findings
            .iter()
            .find(|f| f.message.contains("findPublic"))
            .unwrap();
        let priv_f = findings
            .iter()
            .find(|f| f.message.contains("findPrivate"))
            .unwrap();
        assert_eq!(pub_f.severity, "warning");
        assert_eq!(priv_f.severity, "info");
    }

    #[test]
    fn test_suppress_warnings() {
        let src = r#"
class Foo {
    @SuppressWarnings("null")
    String find() { return null; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_overlap_with_catch() {
        let src = r#"
class Foo {
    Object m() {
        try { return new Object(); }
        catch (Exception e) { return null; }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_optional_return_null_is_error() {
        let src = "class Foo { public Optional<String> find() { return null; } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "null_return_optional");
        assert_eq!(findings[0].severity, "error");
        assert!(findings[0].message.contains("Optional"));
        assert!(findings[0].message.contains("Optional.empty()"));
    }

    #[test]
    fn test_ternary_null_detected() {
        let src = r#"
class Foo {
    public String find(String x) { return x != null ? x : null; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("find"));
    }

    #[test]
    fn test_primitive_return_type_skipped() {
        let src = "class Foo { public int getCount() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_void_return_type_skipped() {
        let src = "class Foo { public void doSomething() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_deprecated_skipped() {
        let src = "class Foo { @Deprecated public String find() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_test_annotation_skipped() {
        let src = "class Foo { @Test public String find() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
