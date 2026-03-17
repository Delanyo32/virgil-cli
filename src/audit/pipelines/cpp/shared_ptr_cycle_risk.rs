use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_field_declaration_query, extract_snippet, find_capture_index, find_identifier_in_declarator,
    is_inside_node_kind, node_text,
};

pub struct SharedPtrCycleRiskPipeline {
    field_query: Arc<Query>,
}

impl SharedPtrCycleRiskPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_declaration_query()?,
        })
    }

    fn is_shared_ptr_type(type_text: &str) -> bool {
        type_text.contains("shared_ptr")
    }
}

impl Pipeline for SharedPtrCycleRiskPipeline {
    fn name(&self) -> &str {
        "shared_ptr_cycle_risk"
    }

    fn description(&self) -> &str {
        "Detects shared_ptr class members that risk reference cycles — consider weak_ptr for back-references"
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
                // Only flag inside class/struct
                if !is_inside_node_kind(decl_cap.node, "class_specifier")
                    && !is_inside_node_kind(decl_cap.node, "struct_specifier")
                {
                    continue;
                }

                let type_text = node_text(type_cap.node, source);

                // Also check the full declaration text since the template type might be in the declarator
                let full_text = node_text(decl_cap.node, source);

                if !Self::is_shared_ptr_type(type_text) && !Self::is_shared_ptr_type(full_text) {
                    continue;
                }

                let var_name = declarator_cap
                    .and_then(|c| find_identifier_in_declarator(c.node, source))
                    .unwrap_or_else(|| "<unknown>".to_string());

                let start = decl_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "shared_ptr_cycle_risk".to_string(),
                    message: format!(
                        "`shared_ptr` member `{var_name}` may create a reference cycle — consider `std::weak_ptr` for back-references"
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
        let pipeline = SharedPtrCycleRiskPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_shared_ptr_member() {
        let src = r#"
class Node {
    std::shared_ptr<Node> next;
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "shared_ptr_cycle_risk");
        assert!(findings[0].message.contains("next"));
    }

    #[test]
    fn no_finding_for_weak_ptr() {
        let src = r#"
class Node {
    std::weak_ptr<Node> parent;
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_for_unique_ptr() {
        let src = r#"
class Node {
    std::unique_ptr<Node> child;
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_shared_ptrs() {
        let src = r#"
class Graph {
    std::shared_ptr<Graph> left;
    std::shared_ptr<Graph> right;
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn no_finding_for_local_variable() {
        let src = r#"
void f() {
    std::shared_ptr<int> p = std::make_shared<int>(42);
}
"#;
        let findings = parse_and_check(src);
        // Not a class member, should not flag
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
class Foo {
    std::shared_ptr<Foo> self_ref;
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "info");
        assert_eq!(findings[0].pipeline, "shared_ptr_cycle_risk");
    }
}
