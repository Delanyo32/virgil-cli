use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, compile_parameter_declaration_query, extract_snippet,
    find_capture_index, is_pointer_declarator, node_text,
};

pub struct VoidPointerAbusePipeline {
    param_query: Arc<Query>,
    fn_def_query: Arc<Query>,
}

impl VoidPointerAbusePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_parameter_declaration_query()?,
            fn_def_query: compile_function_definition_query()?,
        })
    }
}

impl Pipeline for VoidPointerAbusePipeline {
    fn name(&self) -> &str {
        "void_pointer_abuse"
    }

    fn description(&self) -> &str {
        "Detects void* parameters and return types that bypass type safety"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Check void* parameters
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.param_query, tree.root_node(), source);

            let param_type_idx = find_capture_index(&self.param_query, "param_type");
            let param_decl_idx = find_capture_index(&self.param_query, "param_decl");
            let param_declarator_idx = find_capture_index(&self.param_query, "param_declarator");

            while let Some(m) = matches.next() {
                let type_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_type_idx);
                let decl_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_decl_idx);
                let declarator_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_declarator_idx);

                if let (Some(type_cap), Some(decl_cap)) = (type_cap, decl_cap) {
                    let type_text = node_text(type_cap.node, source).trim();

                    // Check: type is "void" and declarator is pointer
                    if type_text != "void" {
                        continue;
                    }

                    let has_pointer = declarator_cap
                        .map(|c| is_pointer_declarator(c.node))
                        .unwrap_or(false);

                    if !has_pointer {
                        continue;
                    }

                    let start = decl_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "void_pointer_parameter".to_string(),
                        message: "void* parameter bypasses type safety — consider using a concrete type or typed callback".to_string(),
                        snippet: extract_snippet(source, decl_cap.node, 1),
                    });
                }
            }
        }

        // Check void* return types
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

            let declarator_idx = find_capture_index(&self.fn_def_query, "declarator");
            let fn_def_idx = find_capture_index(&self.fn_def_query, "fn_def");

            while let Some(m) = matches.next() {
                let decl_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == declarator_idx);
                let fn_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_def_idx);

                if let (Some(decl_cap), Some(fn_cap)) = (decl_cap, fn_cap) {
                    let fn_node = fn_cap.node;

                    // Check if return type is void and declarator starts with pointer
                    let type_node = fn_node.child_by_field_name("type");
                    let type_text = type_node
                        .map(|n| node_text(n, source).trim().to_string())
                        .unwrap_or_default();

                    if type_text != "void" {
                        continue;
                    }

                    // The top-level declarator should be pointer_declarator
                    if decl_cap.node.kind() != "pointer_declarator" {
                        continue;
                    }

                    let start = fn_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "void_pointer_parameter".to_string(),
                        message: "function returns void* — callers must cast, which bypasses type safety".to_string(),
                        snippet: extract_snippet(source, fn_cap.node, 1),
                    });
                }
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
        let pipeline = VoidPointerAbusePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_void_pointer_param() {
        let src = "void process(void *data) {}";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "void_pointer_parameter"));
    }

    #[test]
    fn detects_void_pointer_return() {
        let src = "void *create() { return 0; }";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.message.contains("returns void*")));
    }

    #[test]
    fn skips_int_pointer_param() {
        let src = "void process(int *data) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_void_no_pointer() {
        let src = "void func(void) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
