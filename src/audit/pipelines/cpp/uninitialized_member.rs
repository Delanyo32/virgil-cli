use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::cpp_primitives::{
    compile_field_declaration_query, extract_snippet,
    find_capture_index, find_identifier_in_declarator, is_inside_node_kind, node_text,
};

const PRIMITIVE_TYPES: &[&str] = &[
    "int", "unsigned", "signed", "short", "long", "float", "double", "char", "bool", "size_t",
    "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t",
    "ptrdiff_t", "intptr_t", "uintptr_t",
];

pub struct UninitializedMemberPipeline {
    field_query: Arc<Query>,
}

impl UninitializedMemberPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_declaration_query()?,
        })
    }

    fn is_primitive_type(type_text: &str) -> bool {
        let trimmed = type_text.trim();
        // Handle multi-word types like "unsigned int", "long long"
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        words.iter().all(|w| PRIMITIVE_TYPES.contains(w) || *w == "unsigned" || *w == "signed" || *w == "long" || *w == "short")
            && !words.is_empty()
    }

    fn has_initializer(field_node: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(field_node, source);
        // Check for = initializer or {} brace initializer
        text.contains('=') || text.contains('{')
    }

    fn has_default_member_init(declarator_node: tree_sitter::Node) -> bool {
        // Check if the declarator has a default_value field or init child
        if declarator_node.child_by_field_name("default_value").is_some() {
            return true;
        }
        let mut cursor = declarator_node.walk();
        for child in declarator_node.children(&mut cursor) {
            if child.kind() == "bitfield_clause" || child.kind() == "initializer_list" {
                return true;
            }
        }
        false
    }
}

impl Pipeline for UninitializedMemberPipeline {
    fn name(&self) -> &str {
        "uninitialized_member"
    }

    fn description(&self) -> &str {
        "Detects uninitialized primitive member variables in classes/structs"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let field_type_idx = find_capture_index(&self.field_query, "field_type");
        let field_decl_idx = find_capture_index(&self.field_query, "field_decl");
        let field_declarator_idx = find_capture_index(&self.field_query, "field_declarator");

        while let Some(m) = matches.next() {
            let type_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_type_idx);
            let decl_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_decl_idx);
            let declarator_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_declarator_idx);

            if let (Some(type_cap), Some(decl_cap)) = (type_cap, decl_cap) {
                // Only flag fields inside class/struct bodies
                if !is_inside_node_kind(decl_cap.node, "class_specifier")
                    && !is_inside_node_kind(decl_cap.node, "struct_specifier")
                {
                    continue;
                }

                let type_text = node_text(type_cap.node, source);

                if !Self::is_primitive_type(type_text) {
                    continue;
                }

                // Check if it has an initializer
                if Self::has_initializer(decl_cap.node, source) {
                    continue;
                }

                if let Some(declarator) = declarator_cap {
                    if Self::has_default_member_init(declarator.node) {
                        continue;
                    }
                }

                let var_name = declarator_cap
                    .and_then(|c| find_identifier_in_declarator(c.node, source))
                    .unwrap_or_else(|| "<unknown>".to_string());

                let start = decl_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "uninitialized_member".to_string(),
                    message: format!(
                        "member `{var_name}` ({type_text}) has no default initializer — may contain indeterminate value"
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UninitializedMemberPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_uninitialized_int() {
        let src = "class Foo { int x; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "uninitialized_member");
        assert!(findings[0].message.contains("x"));
    }

    #[test]
    fn skips_initialized_member() {
        let src = "class Foo { int x = 0; };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_brace_initialized() {
        let src = "class Foo { int x{0}; };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_primitive_member() {
        let src = "class Foo { std::string name; };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_in_struct() {
        let src = "struct Bar { double val; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_multiple_uninitialized() {
        let src = "class Foo { int x; float y; bool z; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 3);
    }

    #[test]
    fn metadata_correct() {
        let src = "class Foo { int x; };";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "uninitialized_member");
    }
}
