use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_field_declaration_query, extract_snippet, find_capture_index,
    find_identifier_in_declarator, is_inside_node_kind, node_text,
};

const PRIMITIVE_TYPES: &[&str] = &[
    "int", "unsigned", "signed", "short", "long", "float", "double", "char", "bool", "size_t",
    "void", "uint8_t", "int8_t", "uint16_t", "int16_t", "uint32_t", "int32_t", "uint64_t",
    "int64_t",
];

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
        let trimmed = type_text.trim();
        // Exact prefix match to avoid matching "my_shared_ptr_wrapper"
        trimmed == "shared_ptr"
            || trimmed == "std::shared_ptr"
            || trimmed.starts_with("shared_ptr<")
            || trimmed.starts_with("std::shared_ptr<")
    }

    fn extract_template_arg(type_text: &str) -> Option<&str> {
        // Extract T from shared_ptr<T> or std::shared_ptr<T>
        let start = type_text.find('<')?;
        let end = type_text.rfind('>')?;
        if end > start + 1 {
            Some(type_text[start + 1..end].trim())
        } else {
            None
        }
    }

    fn is_primitive_template_arg(arg: &str) -> bool {
        PRIMITIVE_TYPES.contains(&arg.trim())
    }

    fn get_enclosing_class_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut current = node.parent();
        while let Some(parent) = current {
            if (parent.kind() == "class_specifier" || parent.kind() == "struct_specifier")
                && let Some(name_node) = parent.child_by_field_name("name") {
                    return Some(node_text(name_node, source).to_string());
                }
            current = parent.parent();
        }
        None
    }
}

impl GraphPipeline for SharedPtrCycleRiskPipeline {
    fn name(&self) -> &str {
        "shared_ptr_cycle_risk"
    }

    fn description(&self) -> &str {
        "Detects shared_ptr class members that risk reference cycles — consider weak_ptr for back-references"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
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
                let full_text = node_text(decl_cap.node, source);

                if !Self::is_shared_ptr_type(type_text) && !Self::is_shared_ptr_type(full_text) {
                    continue;
                }

                // Extract template argument
                let template_arg = Self::extract_template_arg(full_text)
                    .or_else(|| Self::extract_template_arg(type_text));

                // Skip shared_ptr<primitive> — no cycle possible
                if let Some(arg) = template_arg
                    && Self::is_primitive_template_arg(arg) {
                        continue;
                    }

                if is_nolint_suppressed(source, decl_cap.node, self.name()) {
                    continue;
                }

                let var_name = declarator_cap
                    .and_then(|c| find_identifier_in_declarator(c.node, source))
                    .unwrap_or_else(|| "<unknown>".to_string());

                // Graduate severity: self-referential is highest risk
                let severity = if let Some(arg) = template_arg {
                    let enclosing = Self::get_enclosing_class_name(decl_cap.node, source);
                    if enclosing.as_deref() == Some(arg) {
                        "warning" // Self-referential: class has shared_ptr to itself
                    } else {
                        "info"
                    }
                } else {
                    "info"
                };

                let start = decl_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SharedPtrCycleRiskPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.cpp",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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
        // Self-referential gets "warning"
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "shared_ptr_cycle_risk");
    }

    #[test]
    fn shared_ptr_int_no_finding() {
        let src = r#"
class Foo {
    std::shared_ptr<int> data;
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn self_referential_warning() {
        let src = r#"
class Node {
    std::shared_ptr<Node> next;
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn nolint_suppression() {
        let src = r#"
class Node {
    std::shared_ptr<Node> next; // NOLINT
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn custom_type_name_not_matched() {
        let src = r#"
class Foo {
    my_shared_ptr_wrapper data;
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
