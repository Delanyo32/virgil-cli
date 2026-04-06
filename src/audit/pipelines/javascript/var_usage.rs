use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{compile_variable_declaration_query, extract_snippet, node_text};

pub struct VarUsagePipeline {
    var_query: Arc<Query>,
}

impl VarUsagePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            var_query: compile_variable_declaration_query()?,
        })
    }
}

impl NodePipeline for VarUsagePipeline {
    fn name(&self) -> &str {
        "var_usage"
    }

    fn description(&self) -> &str {
        "Detects `var` declarations that should use `let` or `const`"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.var_query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.first() {
                let node = cap.node;

                // NOLINT suppression check
                if is_nolint_suppressed(source, node, self.name()) {
                    continue;
                }

                let start = node.start_position();

                // Check if parent is for_statement (var in for loop initializer)
                let is_for_loop = node
                    .parent()
                    .map(|p| p.kind() == "for_statement")
                    .unwrap_or(false);

                if is_for_loop {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "var_in_for_loop".to_string(),
                        message:
                            "use `let` instead of `var` in for loop — `var` leaks to function scope"
                                .to_string(),
                        snippet: extract_snippet(source, node, 1),
                    });
                } else {
                    // Determine if const or let is appropriate by checking for reassignment
                    let suggestion = suggest_replacement(node, tree, source);
                    let message = format!(
                        "`var` has function scope and hoisting — prefer `{suggestion}`"
                    );

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "var_usage".to_string(),
                        message,
                        snippet: extract_snippet(source, node, 1),
                    });
                }
            }
        }

        findings
    }
}

/// Determine whether a `var` declaration should become `const` or `let`.
///
/// Extracts the declared variable name from the first `variable_declarator` child,
/// then walks the enclosing scope (function body or program) looking for
/// `assignment_expression` nodes whose left-hand side matches that name.
/// Returns `"const"` if no reassignment is found, `"let"` otherwise.
fn suggest_replacement(
    var_decl_node: tree_sitter::Node,
    tree: &tree_sitter::Tree,
    source: &[u8],
) -> &'static str {
    // Extract declared variable name from first variable_declarator's name field
    let var_name = var_decl_node
        .named_child(0)
        .filter(|child| child.kind() == "variable_declarator")
        .and_then(|declarator| declarator.child_by_field_name("name"))
        .map(|name_node| node_text(name_node, source));

    let var_name = match var_name {
        Some(name) => name,
        None => return "let", // fallback if we can't extract the name
    };

    // Walk up to the enclosing scope (function body or program)
    let scope = find_enclosing_scope(var_decl_node, tree);

    // Search scope for assignment_expression where LHS matches var_name
    if has_reassignment(scope, source, var_name) {
        "let"
    } else {
        "const"
    }
}

/// Find the enclosing scope node: walk up parents until we find a function body
/// (statement_block inside a function) or program root.
fn find_enclosing_scope<'a>(node: tree_sitter::Node<'a>, tree: &'a tree_sitter::Tree) -> tree_sitter::Node<'a> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "function_declaration" | "function_expression" | "arrow_function" | "method_definition" | "program" => {
                // For functions, use the body (statement_block); for program, use as-is
                if parent.kind() == "program" {
                    return parent;
                }
                if let Some(body) = parent.child_by_field_name("body") {
                    return body;
                }
                return parent;
            }
            _ => current = parent.parent(),
        }
    }
    tree.root_node()
}

/// Walk a scope node recursively looking for `assignment_expression` where
/// the LHS identifier text matches `var_name`.
fn has_reassignment(scope: tree_sitter::Node, source: &[u8], var_name: &str) -> bool {
    let mut cursor = scope.walk();
    walk_tree_for_assignment(scope, &mut cursor, source, var_name)
}

fn walk_tree_for_assignment(
    node: tree_sitter::Node,
    cursor: &mut tree_sitter::TreeCursor,
    source: &[u8],
    var_name: &str,
) -> bool {
    if node.kind() == "assignment_expression" {
        if let Some(lhs) = node.child_by_field_name("left") {
            if lhs.kind() == "identifier" && node_text(lhs, source) == var_name {
                return true;
            }
        }
    }

    // Also check update_expression (x++, ++x, x--, --x)
    if node.kind() == "update_expression" {
        if let Some(arg) = node.named_child(0) {
            if arg.kind() == "identifier" && node_text(arg, source) == var_name {
                return true;
            }
        }
    }

    // Also check augmented_assignment (+=, -=, etc.)
    if node.kind() == "augmented_assignment_expression" {
        if let Some(lhs) = node.child_by_field_name("left") {
            if lhs.kind() == "identifier" && node_text(lhs, source) == var_name {
                return true;
            }
        }
    }

    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if walk_tree_for_assignment(child, cursor, source, var_name) {
                // Reset cursor position before returning
                while cursor.goto_parent() {}
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }

    false
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
        let pipeline = VarUsagePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_var_declaration() {
        let findings = parse_and_check("var x = 1;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "var_usage");
    }

    #[test]
    fn skips_let_declaration() {
        let findings = parse_and_check("let x = 1;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_const_declaration() {
        let findings = parse_and_check("const x = 1;");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_vars() {
        let findings = parse_and_check("var x = 1;\nvar y = 2;");
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn var_in_for_loop() {
        let findings = parse_and_check("for (var i = 0; i < 10; i++) {}");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "var_in_for_loop");
    }

    #[test]
    fn var_suggest_const() {
        let findings = parse_and_check("function f() { var x = 1; return x; }");
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].message.contains("const"),
            "Expected message to suggest const, got: {}",
            findings[0].message
        );
    }

    #[test]
    fn var_suggest_let() {
        let findings = parse_and_check("function f() { var x = 1; x = 2; return x; }");
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].message.contains("let"),
            "Expected message to suggest let, got: {}",
            findings[0].message
        );
    }

    #[test]
    fn nolint_suppresses() {
        let findings = parse_and_check("// NOLINT(var_usage)\nvar x = 1;");
        assert_eq!(findings.len(), 0);
    }
}
