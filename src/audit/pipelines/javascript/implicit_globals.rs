use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_assignment_expression_query, extract_snippet, find_capture_index, node_text,
};

const KNOWN_GLOBALS: &[&str] = &[
    "window",
    "document",
    "module",
    "exports",
    "require",
    "process",
    "Buffer",
    "__dirname",
    "__filename",
    "global",
    "globalThis",
    "console",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "undefined",
    "NaN",
    "Infinity",
];

pub struct ImplicitGlobalsPipeline {
    assign_query: Arc<Query>,
}

impl ImplicitGlobalsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            assign_query: compile_assignment_expression_query()?,
        })
    }

    /// Walk up from `node` to enclosing function/program scope and check
    /// if there's a variable_declarator with the same name.
    fn has_declaration_in_scope(node: tree_sitter::Node, name: &str, source: &[u8]) -> bool {
        let mut scope = node.parent();
        while let Some(s) = scope {
            match s.kind() {
                "function_declaration" | "function_expression" | "arrow_function" | "program" => {
                    return Self::scope_has_declaration(s, name, source);
                }
                _ => {
                    scope = s.parent();
                }
            }
        }
        false
    }

    fn scope_has_declaration(scope_node: tree_sitter::Node, name: &str, source: &[u8]) -> bool {
        // Walk all descendants looking for variable_declarator or formal_parameters
        let mut cursor = scope_node.walk();
        Self::search_declarations(scope_node, &mut cursor, name, source)
    }

    fn search_declarations(
        node: tree_sitter::Node,
        cursor: &mut tree_sitter::TreeCursor,
        name: &str,
        source: &[u8],
    ) -> bool {
        match node.kind() {
            "variable_declarator" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if name_node.kind() == "identifier" && node_text(name_node, source) == name {
                        return true;
                    }
                }
            }
            "formal_parameters" => {
                let mut child_cursor = node.walk();
                for child in node.named_children(&mut child_cursor) {
                    if child.kind() == "identifier" && node_text(child, source) == name {
                        return true;
                    }
                }
            }
            _ => {}
        }

        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            // Don't recurse into nested functions — they have their own scope
            if child.kind() == "function_declaration"
                || child.kind() == "function"
                || child.kind() == "arrow_function"
            {
                continue;
            }
            if Self::search_declarations(child, cursor, name, source) {
                return true;
            }
        }

        false
    }
}

impl Pipeline for ImplicitGlobalsPipeline {
    fn name(&self) -> &str {
        "implicit_globals"
    }

    fn description(&self) -> &str {
        "Detects assignments to undeclared variables that create implicit globals"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.assign_query, tree.root_node(), source);

        let lhs_idx = find_capture_index(&self.assign_query, "lhs");
        let assign_idx = find_capture_index(&self.assign_query, "assign");

        while let Some(m) = matches.next() {
            let lhs_cap = m.captures.iter().find(|c| c.index as usize == lhs_idx);
            let assign_cap = m.captures.iter().find(|c| c.index as usize == assign_idx);

            if let (Some(lhs), Some(assign)) = (lhs_cap, assign_cap) {
                // Only flag bare identifier assignments (not member_expression like obj.prop = ...)
                if lhs.node.kind() != "identifier" {
                    continue;
                }

                let name = node_text(lhs.node, source);

                if KNOWN_GLOBALS.contains(&name) {
                    continue;
                }

                if Self::has_declaration_in_scope(assign.node, name, source) {
                    continue;
                }

                let start = assign.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "implicit_global".to_string(),
                    message: format!(
                        "assignment to `{name}` without declaration — creates an implicit global"
                    ),
                    snippet: extract_snippet(source, assign.node, 1),
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
        let pipeline = ImplicitGlobalsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_implicit_global() {
        let src = "function foo() { x = 42; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "implicit_global");
        assert!(findings[0].message.contains("x"));
    }

    #[test]
    fn skips_declared_variable() {
        let src = "function foo() { let x; x = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_known_globals() {
        let src = "function foo() { module = {}; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_member_expression() {
        let src = "function foo() { obj.prop = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_parameter() {
        let src = "function foo(x) { x = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
