use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_field_declaration_query, extract_snippet, find_capture_index,
    find_identifier_in_declarator, has_storage_class, is_inside_node_kind, is_pointer_declarator,
    node_text,
};

const PRIMITIVE_TYPES: &[&str] = &[
    "int",
    "unsigned",
    "signed",
    "short",
    "long",
    "float",
    "double",
    "char",
    "bool",
    "size_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "ptrdiff_t",
    "intptr_t",
    "uintptr_t",
    "wchar_t",
    "char16_t",
    "char32_t",
    "char8_t",
    "ssize_t",
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
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        words.iter().all(|w| {
            PRIMITIVE_TYPES.contains(w)
                || *w == "unsigned"
                || *w == "signed"
                || *w == "long"
                || *w == "short"
                || *w == "volatile"
                || *w == "const"
        }) && !words.is_empty()
    }

    fn has_initializer(declarator_node: tree_sitter::Node) -> bool {
        // Check for default_value field or init-related children via AST
        if declarator_node
            .child_by_field_name("default_value")
            .is_some()
        {
            return true;
        }
        let mut cursor = declarator_node.walk();
        for child in declarator_node.children(&mut cursor) {
            if child.kind() == "bitfield_clause"
                || child.kind() == "initializer_list"
                || child.kind() == "initializer_pair"
            {
                return true;
            }
        }
        false
    }

    fn field_has_equals_init(field_node: tree_sitter::Node) -> bool {
        // Check if the field_declaration has a default_value field (NSDMI: int x = 0;)
        if field_node.child_by_field_name("default_value").is_some() {
            return true;
        }
        let mut cursor = field_node.walk();
        for child in field_node.children(&mut cursor) {
            // init_declarator wraps `identifier = value` in some grammars
            if child.kind() == "init_declarator" {
                return true;
            }
            // Brace init: field_declaration with initializer_list child
            if child.kind() == "initializer_list" {
                return true;
            }
            // Check recursively in declarator children for default_value
            if child.child_by_field_name("default_value").is_some() {
                return true;
            }
        }
        false
    }

    fn is_initialized_in_constructors(
        member_name: &str,
        class_body: tree_sitter::Node,
        class_name: &str,
        source: &[u8],
    ) -> bool {
        // Find all constructors in the class body and check member initializer lists
        let mut has_constructor = false;
        let mut all_constructors_init = true;

        let mut cursor = class_body.walk();
        for child in class_body.children(&mut cursor) {
            if child.kind() != "function_definition" {
                continue;
            }

            // Check if this is a constructor (name matches class name, not a destructor)
            if let Some(declarator) = child.child_by_field_name("declarator") {
                if let Some(name) = find_identifier_in_declarator(declarator, source) {
                    if name != class_name {
                        continue;
                    }
                } else {
                    continue;
                }
            } else {
                continue;
            }

            // It's a constructor
            has_constructor = true;

            // Check the member initializer list (field_initializer_list)
            let mut found_init = false;
            let mut inner_cursor = child.walk();
            for inner_child in child.children(&mut inner_cursor) {
                if inner_child.kind() == "field_initializer_list" {
                    let init_text = node_text(inner_child, source);
                    if init_text.contains(member_name) {
                        found_init = true;
                        break;
                    }
                }
            }

            if !found_init {
                all_constructors_init = false;
            }
        }

        has_constructor && all_constructors_init
    }
}

impl GraphPipeline for UninitializedMemberPipeline {
    fn name(&self) -> &str {
        "uninitialized_member"
    }

    fn description(&self) -> &str {
        "Detects uninitialized primitive member variables in classes/structs"
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
                // Only flag fields inside class/struct bodies
                if !is_inside_node_kind(decl_cap.node, "class_specifier")
                    && !is_inside_node_kind(decl_cap.node, "struct_specifier")
                {
                    continue;
                }

                // Skip static members
                if has_storage_class(decl_cap.node, source, "static") {
                    continue;
                }

                let type_text = node_text(type_cap.node, source);

                // Check for pointer members (always risky when uninitialized)
                let is_pointer = declarator_cap
                    .map(|c| is_pointer_declarator(c.node))
                    .unwrap_or(false);

                if !Self::is_primitive_type(type_text) && !is_pointer {
                    continue;
                }

                // Check for AST-based initializer (not string matching)
                if Self::field_has_equals_init(decl_cap.node) {
                    continue;
                }

                if let Some(declarator) = declarator_cap
                    && Self::has_initializer(declarator.node) {
                        continue;
                    }

                let var_name = declarator_cap
                    .and_then(|c| find_identifier_in_declarator(c.node, source))
                    .unwrap_or_else(|| "<unknown>".to_string());

                // Check if all constructors initialize this member
                if let Some(class_body) = decl_cap.node.parent()
                    && class_body.kind() == "field_declaration_list"
                        && let Some(class_node) = class_body.parent() {
                            let class_name_node = class_node.child_by_field_name("name");
                            if let Some(cn) = class_name_node {
                                let class_name = node_text(cn, source);
                                if Self::is_initialized_in_constructors(
                                    &var_name,
                                    class_body,
                                    class_name,
                                    source,
                                ) {
                                    continue;
                                }
                            }
                        }

                if is_nolint_suppressed(source, decl_cap.node, self.name()) {
                    continue;
                }

                // Pointer members get higher severity
                let severity = if is_pointer { "error" } else { "warning" };

                let start = decl_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UninitializedMemberPipeline::new().unwrap();
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

    #[test]
    fn skips_static_member() {
        let src = "class Foo { static int count; };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn initialized_in_constructor() {
        let src = r#"
class Foo {
    int x;
    Foo() : x(0) {}
};
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression() {
        let src = "class Foo { int x; // NOLINT\n};";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn pointer_member_error_severity() {
        let src = "class Foo { int* ptr; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }
}
