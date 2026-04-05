use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_union_specifier_query, extract_snippet, find_capture_index, is_inside_node_kind,
    node_text,
};

const NON_TRIVIAL_TYPES: &[&str] = &[
    "string", "vector", "map", "unordered_map", "set", "unordered_set", "list", "deque",
    "shared_ptr", "unique_ptr", "optional", "any", "function", "variant",
];

pub struct RawUnionPipeline {
    union_query: Arc<Query>,
}

impl RawUnionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            union_query: compile_union_specifier_query()?,
        })
    }

    fn has_non_trivial_member(union_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Look for the field_declaration_list body
        let mut cursor = union_node.walk();
        for child in union_node.children(&mut cursor) {
            if child.kind() == "field_declaration_list" {
                let mut inner_cursor = child.walk();
                for field in child.children(&mut inner_cursor) {
                    if field.kind() == "field_declaration" {
                        let field_text = node_text(field, source);
                        for nt in NON_TRIVIAL_TYPES {
                            if field_text.contains(nt) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    fn is_inside_tagged_union(union_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Check if sibling declarations in the parent struct contain an enum
        if let Some(parent) = union_node.parent()
            && parent.kind() == "field_declaration_list" {
                let mut cursor = parent.walk();
                for sibling in parent.children(&mut cursor) {
                    if sibling.kind() == "field_declaration" {
                        let text = node_text(sibling, source);
                        if text.contains("enum") {
                            return true;
                        }
                    }
                }
            }
        false
    }
}

impl NodePipeline for RawUnionPipeline {
    fn name(&self) -> &str {
        "raw_union"
    }

    fn description(&self) -> &str {
        "Detects raw union usage — prefer std::variant for type-safe tagged unions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.union_query, tree.root_node(), source);

        let union_def_idx = find_capture_index(&self.union_query, "union_def");
        let union_name_idx = find_capture_index(&self.union_query, "union_name");

        while let Some(m) = matches.next() {
            let union_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == union_def_idx);

            if let Some(union_cap) = union_cap {
                let name_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == union_name_idx);
                let name = name_cap
                    .and_then(|c| c.node.utf8_text(source).ok())
                    .unwrap_or("<anonymous>");
                let is_anonymous = name_cap.is_none();

                // Skip anonymous unions inside class/struct (idiomatic C++ pattern)
                if is_anonymous
                    && (is_inside_node_kind(union_cap.node, "class_specifier")
                        || is_inside_node_kind(union_cap.node, "struct_specifier"))
                {
                    continue;
                }

                // Skip unions in extern "C" blocks (C interop)
                if is_inside_node_kind(union_cap.node, "linkage_specification") {
                    continue;
                }

                if is_nolint_suppressed(source, union_cap.node, self.name()) {
                    continue;
                }

                // Graduate severity based on member types
                let severity = if Self::has_non_trivial_member(union_cap.node, source) {
                    "warning"
                } else {
                    "info"
                };

                let mut message = format!(
                    "raw `union {name}` — consider using `std::variant` for type safety"
                );
                if Self::is_inside_tagged_union(union_cap.node, source) {
                    message.push_str(" (tagged union detected)");
                }

                let start = union_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "raw_union".to_string(),
                    message,
                    snippet: extract_snippet(source, union_cap.node, 1),
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
        let pipeline = RawUnionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_named_union() {
        let src = "union Data { int i; float f; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_union");
        assert!(findings[0].message.contains("Data"));
    }

    #[test]
    fn detects_anonymous_union() {
        let src = "union { int x; float y; } val;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_union");
    }

    #[test]
    fn no_findings_without_union() {
        let src = "struct Foo { int x; float y; };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_unions() {
        let src = r#"
union A { int i; float f; };
union B { char c; double d; };
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn anonymous_union_in_struct_skipped() {
        let src = "struct Foo { union { int i; float f; }; };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn extern_c_union_skipped() {
        let src = r#"extern "C" { union Data { int i; float f; }; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression() {
        let src = "union Data { int i; float f; }; // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn union_with_nontrivial_members_warning() {
        let src = r#"
union Bad {
    std::string s;
    int i;
};
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }
}
