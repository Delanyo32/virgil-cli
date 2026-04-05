use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::has_suppress_warnings;

use super::primitives::{
    compile_assignment_query, compile_method_invocation_with_object_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct StringConcatInLoopsPipeline {
    assignment_query: Arc<Query>,
    method_invocation_query: Arc<Query>,
}

impl StringConcatInLoopsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            assignment_query: compile_assignment_query()?,
            method_invocation_query: compile_method_invocation_with_object_query()?,
        })
    }
}

/// Walk up from a node to find the declared type of a local variable in the enclosing method.
fn resolve_variable_type<'a>(
    var_name: &str,
    node: tree_sitter::Node<'a>,
    source: &'a [u8],
) -> Option<String> {
    // Walk up to the enclosing method/constructor body
    let mut scope = node.parent();
    while let Some(p) = scope {
        if p.kind() == "method_declaration" || p.kind() == "constructor_declaration" {
            if let Some(body) = p.child_by_field_name("body") {
                scope = Some(body);
            }
            break;
        }
        scope = p.parent();
    }
    let scope = scope?;
    find_var_type_in_scope(scope, var_name, source)
}

fn find_var_type_in_scope(
    node: tree_sitter::Node,
    var_name: &str,
    source: &[u8],
) -> Option<String> {
    if node.kind() == "local_variable_declaration" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator"
                && let Some(name_node) = child.child_by_field_name("name")
                && node_text(name_node, source) == var_name
                && let Some(type_node) = node.child_by_field_name("type")
            {
                return Some(node_text(type_node, source).to_string());
            }
        }
    }
    // Recurse into children but skip nested methods/classes
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "method_declaration" || child.kind() == "class_declaration" {
            continue;
        }
        if let Some(t) = find_var_type_in_scope(child, var_name, source) {
            return Some(t);
        }
    }
    None
}

/// Walk up from a node to find its enclosing method_declaration.
fn enclosing_method(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut p = node.parent();
    while let Some(n) = p {
        if n.kind() == "method_declaration" || n.kind() == "constructor_declaration" {
            return Some(n);
        }
        p = n.parent();
    }
    None
}

impl GraphPipeline for StringConcatInLoopsPipeline {
    fn name(&self) -> &str {
        "string_concat_in_loops"
    }

    fn description(&self) -> &str {
        "Detects string concatenation with += inside loops — use StringBuilder instead"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        let mut findings = Vec::new();

        // --- Pass 1: += operator inside loops ---
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.assignment_query, tree.root_node(), source);

            let assign_idx = find_capture_index(&self.assignment_query, "assign");
            let lhs_idx = find_capture_index(&self.assignment_query, "lhs");
            let rhs_idx = find_capture_index(&self.assignment_query, "rhs");

            while let Some(m) = matches.next() {
                let assign_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == assign_idx)
                    .map(|c| c.node);
                let lhs_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == lhs_idx)
                    .map(|c| c.node);
                let rhs_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == rhs_idx)
                    .map(|c| c.node);

                if let (Some(assign_node), Some(rhs_node)) = (assign_node, rhs_node) {
                    // Check if operator is +=
                    let assign_text = node_text(assign_node, source);
                    if !assign_text.contains("+=") {
                        continue;
                    }

                    // Check if inside a loop
                    if !is_inside_loop(assign_node) {
                        continue;
                    }

                    // Check @SuppressWarnings on enclosing method
                    if let Some(m) = enclosing_method(assign_node)
                        && has_suppress_warnings(m, source, "string-concat")
                    {
                        continue;
                    }

                    // Resolve variable type from declaration
                    if let Some(lhs) = lhs_node {
                        let lhs_name = node_text(lhs, source);

                        let var_type = resolve_variable_type(lhs_name, assign_node, source);
                        let is_string_type = var_type.as_deref() == Some("String");
                        let rhs_has_string = contains_string_literal(rhs_node, source);

                        // Flag if: type is String, OR type is unresolved but RHS has string literal
                        if !is_string_type && !rhs_has_string {
                            continue;
                        }
                    } else {
                        // No LHS captured — fall back to RHS-only heuristic
                        if !contains_string_literal(rhs_node, source) {
                            continue;
                        }
                    }

                    let start = assign_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "string_concat_in_loop".to_string(),
                        message:
                            "string concatenation with += inside a loop — use StringBuilder instead"
                                .to_string(),
                        snippet: extract_snippet(source, assign_node, 3),
                    });
                }
            }
        }

        // --- Pass 2: plain concatenation str = str + "x" inside loops ---
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.assignment_query, tree.root_node(), source);

            let assign_idx = find_capture_index(&self.assignment_query, "assign");
            let lhs_idx = find_capture_index(&self.assignment_query, "lhs");
            let rhs_idx = find_capture_index(&self.assignment_query, "rhs");

            while let Some(m) = matches.next() {
                let assign_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == assign_idx)
                    .map(|c| c.node);
                let lhs_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == lhs_idx)
                    .map(|c| c.node);
                let rhs_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == rhs_idx)
                    .map(|c| c.node);

                if let (Some(assign_node), Some(lhs), Some(rhs)) = (assign_node, lhs_node, rhs_node)
                {
                    // Only plain = assignments (skip +=, -=, etc.)
                    let assign_text = node_text(assign_node, source);
                    if assign_text.contains("+=")
                        || assign_text.contains("-=")
                        || assign_text.contains("*=")
                        || assign_text.contains("/=")
                    {
                        continue;
                    }

                    if !is_inside_loop(assign_node) {
                        continue;
                    }

                    // Check @SuppressWarnings on enclosing method
                    if let Some(m) = enclosing_method(assign_node)
                        && has_suppress_warnings(m, source, "string-concat")
                    {
                        continue;
                    }

                    // RHS must be a binary_expression containing the LHS identifier and a string literal
                    if rhs.kind() != "binary_expression" {
                        continue;
                    }

                    let lhs_name = node_text(lhs, source);
                    if !contains_identifier(rhs, source, lhs_name) {
                        continue;
                    }
                    if !contains_string_literal(rhs, source) {
                        continue;
                    }

                    let start = assign_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "string_concat_in_loop".to_string(),
                        message:
                            "string concatenation with + inside a loop — use StringBuilder instead"
                                .to_string(),
                        snippet: extract_snippet(source, assign_node, 3),
                    });
                }
            }
        }

        // --- Pass 3: String.concat() inside loops ---
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.method_invocation_query, tree.root_node(), source);

            let invocation_idx = find_capture_index(&self.method_invocation_query, "invocation");
            let method_name_idx = find_capture_index(&self.method_invocation_query, "method_name");
            let args_idx = find_capture_index(&self.method_invocation_query, "args");

            while let Some(m) = matches.next() {
                let invocation_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == invocation_idx)
                    .map(|c| c.node);
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

                if let (Some(inv), Some(name), Some(args)) = (invocation_node, name_node, args_node)
                {
                    if node_text(name, source) != "concat" {
                        continue;
                    }

                    if !is_inside_loop(inv) {
                        continue;
                    }

                    // Check @SuppressWarnings on enclosing method
                    if let Some(m) = enclosing_method(inv)
                        && has_suppress_warnings(m, source, "string-concat")
                    {
                        continue;
                    }

                    // Check if arguments contain a string literal
                    if !contains_string_literal(args, source) {
                        continue;
                    }

                    let start = inv.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "string_concat_in_loop".to_string(),
                        message:
                            "String.concat() inside a loop — use StringBuilder instead".to_string(),
                        snippet: extract_snippet(source, inv, 3),
                    });
                }
            }
        }

        findings
    }
}

fn is_inside_loop(node: tree_sitter::Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" => {
                return true;
            }
            "method_declaration" | "constructor_declaration" | "class_declaration" => return false,
            _ => parent = p.parent(),
        }
    }
    false
}

fn contains_string_literal(node: tree_sitter::Node, _source: &[u8]) -> bool {
    if node.kind() == "string_literal" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_string_literal(child, _source) {
            return true;
        }
    }
    false
}

/// Check if a node subtree contains an identifier with the given name.
fn contains_identifier(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
    if node.kind() == "identifier" && node_text(node, source) == name {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_identifier(child, source, name) {
            return true;
        }
    }
    false
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
        let pipeline = StringConcatInLoopsPipeline::new().unwrap();
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
    fn detects_string_concat_in_for_loop() {
        let src = r#"
class Foo {
    void m() {
        String s = "";
        for (int i = 0; i < 10; i++) {
            s += "item";
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "string_concat_in_loop");
    }

    #[test]
    fn clean_outside_loop() {
        let src = r#"
class Foo {
    void m() {
        String s = "";
        s += "x";
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_in_while_loop() {
        let src = r#"
class Foo {
    void m() {
        String s = "";
        int i = 0;
        while (i < 10) {
            s += "item";
            i++;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn test_type_resolution_skips_numeric() {
        let src = r#"
class Foo {
    void m() {
        int count = 0;
        for (int i = 0; i < 10; i++) {
            count += 1;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_plain_concatenation() {
        let src = r#"
class Foo {
    void m() {
        String str = "";
        for (int i = 0; i < 10; i++) {
            str = str + "x";
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn test_enhanced_for() {
        let src = r#"
class Foo {
    void m(String[] items) {
        String result = "";
        for (String s : items) {
            result += s;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn test_suppress_warnings() {
        let src = r#"
class Foo {
    @SuppressWarnings("string-concat")
    void m() {
        String s = "";
        for (int i = 0; i < 10; i++) {
            s += "item";
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
