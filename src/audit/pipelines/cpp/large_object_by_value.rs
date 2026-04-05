use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_parameter_declaration_query, extract_snippet, find_capture_index,
    is_pointer_declarator, is_reference_declarator, node_text,
};

const LARGE_TYPES: &[&str] = &[
    "string",
    "std::string",
    "vector",
    "std::vector",
    "map",
    "std::map",
    "unordered_map",
    "std::unordered_map",
    "set",
    "std::set",
    "unordered_set",
    "std::unordered_set",
    "list",
    "std::list",
    "deque",
    "std::deque",
    "array",
    "std::array",
];

pub struct LargeObjectByValuePipeline {
    param_query: Arc<Query>,
}

impl LargeObjectByValuePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_parameter_declaration_query()?,
        })
    }

    fn is_large_type(type_text: &str) -> bool {
        let base = type_text.split('<').next().unwrap_or(type_text).trim();
        LARGE_TYPES.contains(&base)
    }
}

impl NodePipeline for LargeObjectByValuePipeline {
    fn name(&self) -> &str {
        "large_object_by_value"
    }

    fn description(&self) -> &str {
        "Detects large objects (string, vector, map, etc.) passed by value — pass by const reference instead"
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
                let type_text = node_text(type_cap.node, source);

                if !Self::is_large_type(type_text) {
                    continue;
                }

                // Skip if passed by reference or pointer
                if let Some(declarator) = declarator_cap
                    && (is_reference_declarator(declarator.node)
                        || is_pointer_declarator(declarator.node))
                {
                    continue;
                }

                // Also check if the type itself contains & (e.g., `const std::string&`)
                let full_text = node_text(decl_cap.node, source);
                if full_text.contains('&') || full_text.contains('*') {
                    continue;
                }

                if is_nolint_suppressed(source, decl_cap.node, self.name()) {
                    continue;
                }

                let start = decl_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "large_object_by_value".to_string(),
                    message: format!(
                        "`{type_text}` passed by value — consider `const {type_text}&` to avoid copying"
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
        let pipeline = LargeObjectByValuePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_string_by_value() {
        let src = "void f(std::string s) {}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "large_object_by_value");
    }

    #[test]
    fn detects_vector_by_value() {
        let src = "void f(std::vector<int> v) {}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_const_ref() {
        let src = "void f(const std::string& s) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_pointer() {
        let src = "void f(std::string* s) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_primitive_types() {
        let src = "void f(int x, double y, char c) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_map_by_value() {
        let src = "void process(std::map<std::string, int> data) {}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn metadata_correct() {
        let src = "void f(std::string s) {}";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "info");
        assert_eq!(findings[0].pipeline, "large_object_by_value");
    }

    #[test]
    fn unique_ptr_by_value_ok() {
        let src = "void take(std::unique_ptr<Foo> p) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn shared_ptr_by_value_ok() {
        let src = "void share(std::shared_ptr<Foo> p) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn custom_type_not_matched() {
        let src = "void f(my_vector v) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression() {
        let src = "void f(std::string s) {} // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
