use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::c_primitives::{
    compile_parameter_declaration_query, extract_snippet, find_capture_index,
    has_type_qualifier, is_pointer_declarator, node_text,
};

pub struct MissingConstPipeline {
    param_query: Arc<Query>,
}

impl MissingConstPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_parameter_declaration_query()?,
        })
    }
}

impl Pipeline for MissingConstPipeline {
    fn name(&self) -> &str {
        "missing_const"
    }

    fn description(&self) -> &str {
        "Detects non-const pointer parameters that could be const for safety and clarity"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
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

                // Skip void* (caught by void_pointer_abuse pipeline)
                if type_text == "void" {
                    continue;
                }

                // Must be a pointer parameter
                let has_pointer = declarator_cap
                    .map(|c| is_pointer_declarator(c.node))
                    .unwrap_or(false);

                if !has_pointer {
                    continue;
                }

                // Skip if already has const qualifier
                if has_type_qualifier(decl_cap.node, source, "const") {
                    continue;
                }

                let start = decl_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "missing_const_param".to_string(),
                    message: "non-const pointer parameter — add `const` if the function does not modify the data".to_string(),
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
        let pipeline = MissingConstPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_non_const_pointer_param() {
        let src = "void process(char *data) {}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_const_param");
    }

    #[test]
    fn skips_const_pointer() {
        let src = "void process(const char *data) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_pointer() {
        let src = "void process(int n) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_void_pointer() {
        let src = "void process(void *data) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
