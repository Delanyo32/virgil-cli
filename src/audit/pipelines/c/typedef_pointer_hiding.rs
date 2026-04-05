use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{compile_type_definition_query, extract_snippet, find_capture_index};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

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
        Self::contains_function_declarator(declarator)
    }

    fn contains_function_declarator(node: tree_sitter::Node) -> bool {
        if node.kind() == "function_declarator" {
            return true;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::contains_function_declarator(inner);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_declarator" {
                return true;
            }
            if child.kind() == "parenthesized_declarator"
                && Self::contains_function_declarator(child)
            {
                return true;
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

    /// Count the depth of nested pointer_declarator nodes.
    fn pointer_depth(node: tree_sitter::Node) -> usize {
        if node.kind() == "pointer_declarator" {
            if let Some(inner) = node.child_by_field_name("declarator") {
                return 1 + Self::pointer_depth(inner);
            }
            return 1;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::pointer_depth(inner);
        }
        0
    }

    /// Check if the typedef type is an opaque pointer pattern:
    /// `typedef struct Foo *FooHandle;` where struct Foo has no body (forward decl).
    fn is_opaque_pointer(type_node: tree_sitter::Node) -> bool {
        // The type node should be a struct_specifier with no body field
        if type_node.kind() == "struct_specifier" {
            return type_node.child_by_field_name("body").is_none();
        }
        false
    }

    /// Check if the typedef has a const qualifier.
    /// In tree-sitter C, `typedef const char *cstring;` has `const` as a
    /// `type_qualifier` child of the `type_definition` node, not part of the `type` field.
    fn has_const_qualifier(typedef_decl: tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = typedef_decl.walk();
        for child in typedef_decl.children(&mut cursor) {
            if child.kind() == "type_qualifier" {
                let text = child.utf8_text(source).unwrap_or("");
                if text == "const" {
                    return true;
                }
            }
        }
        false
    }
}

impl GraphPipeline for TypedefPointerHidingPipeline {
    fn name(&self) -> &str {
        "typedef_pointer_hiding"
    }

    fn description(&self) -> &str {
        "Detects typedefs that hide pointer types, making ownership and nullability unclear"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.typedef_query, tree.root_node(), source);

        let typedef_type_idx = find_capture_index(&self.typedef_query, "typedef_type");
        let typedef_name_idx = find_capture_index(&self.typedef_query, "typedef_name");
        let typedef_decl_idx = find_capture_index(&self.typedef_query, "typedef_decl");

        while let Some(m) = matches.next() {
            let type_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == typedef_type_idx);
            let name_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == typedef_name_idx);
            let decl_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == typedef_decl_idx);

            if let (Some(type_cap), Some(name_cap), Some(decl_cap)) =
                (type_cap, name_cap, decl_cap)
            {
                let declarator = name_cap.node;
                let type_node = type_cap.node;

                if !Self::has_pointer(declarator) {
                    continue;
                }

                if Self::is_function_pointer_typedef(declarator) {
                    continue;
                }

                // Skip opaque pointer pattern (typedef struct Impl *Handle)
                if Self::is_opaque_pointer(type_node) {
                    continue;
                }

                // Skip const pointer typedefs (typedef const char *cstring)
                if Self::has_const_qualifier(decl_cap.node, source) {
                    continue;
                }

                if is_nolint_suppressed(source, decl_cap.node, self.name()) {
                    continue;
                }

                let typedef_name = declarator.utf8_text(source).unwrap_or("");
                let depth = Self::pointer_depth(declarator);
                let start = decl_cap.node.start_position();

                let (severity, message) = if depth >= 2 {
                    (
                        "warning",
                        format!(
                            "typedef `{typedef_name}` hides {depth} levels of indirection — makes ownership and nullability very unclear"
                        ),
                    )
                } else {
                    (
                        "info",
                        format!(
                            "typedef `{typedef_name}` hides a pointer — makes ownership and nullability unclear"
                        ),
                    )
                };

                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "typedef_hides_pointer".to_string(),
                    message,
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TypedefPointerHidingPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.c",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_pointer_typedef() {
        let src = "typedef int *IntPtr;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "typedef_hides_pointer");
        assert_eq!(findings[0].severity, "info");
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

    #[test]
    fn skips_opaque_pointer() {
        let src = "typedef struct Impl *Handle;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_const_pointer_typedef() {
        let src = "typedef const char *cstring;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn double_pointer_higher_severity() {
        let src = "typedef int **IntPtrPtr;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
        assert!(findings[0].message.contains("2 levels"));
    }

    #[test]
    fn triple_pointer_reports_depth() {
        let src = "typedef int ***TriplePtr;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
        assert!(findings[0].message.contains("3 levels"));
    }

    #[test]
    fn nolint_suppresses() {
        let src = "typedef int *IntPtr; // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
