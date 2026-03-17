use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::c_primitives::{compile_type_definition_query, extract_snippet, find_capture_index};

pub struct TypedefPointerHidingPipeline {
    typedef_query: Arc<Query>,
}

impl TypedefPointerHidingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            typedef_query: compile_type_definition_query()?,
        })
    }

    fn is_function_pointer_typedef(declarator: tree_sitter::Node) -> bool {
        // A function pointer typedef has a pointer_declarator wrapping a function_declarator
        // e.g. typedef void (*Callback)(int);
        Self::contains_function_declarator(declarator)
    }

    fn contains_function_declarator(node: tree_sitter::Node) -> bool {
        if node.kind() == "function_declarator" {
            return true;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::contains_function_declarator(inner);
        }
        // Also check parenthesized_declarator children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_declarator" {
                return true;
            }
            if child.kind() == "parenthesized_declarator" {
                if Self::contains_function_declarator(child) {
                    return true;
                }
            }
        }
        false
    }

    fn has_pointer(node: tree_sitter::Node) -> bool {
        if node.kind() == "pointer_declarator" {
            return true;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::has_pointer(inner);
        }
        false
    }
}

impl Pipeline for TypedefPointerHidingPipeline {
    fn name(&self) -> &str {
        "typedef_pointer_hiding"
    }

    fn description(&self) -> &str {
        "Detects typedefs that hide pointer types, making ownership and nullability unclear"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.typedef_query, tree.root_node(), source);

        let typedef_name_idx = find_capture_index(&self.typedef_query, "typedef_name");
        let typedef_decl_idx = find_capture_index(&self.typedef_query, "typedef_decl");

        while let Some(m) = matches.next() {
            let name_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == typedef_name_idx);
            let decl_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == typedef_decl_idx);

            if let (Some(name_cap), Some(decl_cap)) = (name_cap, decl_cap) {
                let declarator = name_cap.node;

                // Check if declarator contains a pointer
                if !Self::has_pointer(declarator) {
                    continue;
                }

                // Exclude function pointer typedefs
                if Self::is_function_pointer_typedef(declarator) {
                    continue;
                }

                let typedef_name = declarator.utf8_text(source).unwrap_or("");
                let start = decl_cap.node.start_position();

                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "typedef_hides_pointer".to_string(),
                    message: format!(
                        "typedef `{typedef_name}` hides a pointer — makes ownership and nullability unclear"
                    ),
                    snippet: extract_snippet(source, decl_cap.node, 1),
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TypedefPointerHidingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_pointer_typedef() {
        let src = "typedef int *IntPtr;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "typedef_hides_pointer");
    }

    #[test]
    fn skips_non_pointer_typedef() {
        let src = "typedef unsigned int uint;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_function_pointer_typedef() {
        let src = "typedef void (*Callback)(int);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
