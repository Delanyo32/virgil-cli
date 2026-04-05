use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{has_annotation, has_suppress_warnings};

use super::primitives::{
    compile_field_decl_query, extract_snippet, find_capture_index, has_modifier, node_text,
};

const COLLECTION_TYPES: &[&str] = &[
    "List",
    "Map",
    "Set",
    "Collection",
    "ArrayList",
    "HashMap",
    "HashSet",
];

pub struct MutablePublicFieldsPipeline {
    field_query: Arc<Query>,
}

impl MutablePublicFieldsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_decl_query()?,
        })
    }
}

fn is_primitive_type(t: &str) -> bool {
    matches!(
        t,
        "int" | "long" | "double" | "float" | "boolean" | "byte" | "short" | "char"
    )
}

fn enclosing_type_decl(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut p = node.parent();
    while let Some(n) = p {
        if n.kind() == "class_declaration" || n.kind() == "record_declaration" {
            return Some(n);
        }
        p = n.parent();
    }
    None
}

impl GraphPipeline for MutablePublicFieldsPipeline {
    fn name(&self) -> &str {
        "mutable_public_fields"
    }

    fn description(&self) -> &str {
        "Detects public/protected non-final fields — use getters/setters or make fields final"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let field_name_idx = find_capture_index(&self.field_query, "field_name");
        let field_decl_idx = find_capture_index(&self.field_query, "field_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_name_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(decl_node)) = (name_node, decl_node) {
                // Skip @SuppressWarnings("mutable-field")
                if has_suppress_warnings(decl_node, source, "mutable-field") {
                    continue;
                }

                // Skip record_declaration enclosing type
                if let Some(class) = enclosing_type_decl(decl_node) {
                    if class.kind() == "record_declaration" {
                        continue;
                    }

                    // Skip framework-annotated classes
                    if has_annotation(class, source, "Entity")
                        || has_annotation(class, source, "Data")
                        || has_annotation(class, source, "Component")
                        || has_annotation(class, source, "Configuration")
                    {
                        continue;
                    }
                }

                // Skip injection-annotated fields
                if has_annotation(decl_node, source, "Inject")
                    || has_annotation(decl_node, source, "Autowired")
                    || has_annotation(decl_node, source, "Value")
                {
                    continue;
                }

                // Skip volatile and transient fields
                let is_volatile = has_modifier(decl_node, source, "volatile");
                let is_transient = has_modifier(decl_node, source, "transient");
                if is_volatile || is_transient {
                    continue;
                }

                let is_public = has_modifier(decl_node, source, "public");
                let is_protected = has_modifier(decl_node, source, "protected");
                let is_final = has_modifier(decl_node, source, "final");

                if (is_public || is_protected) && !is_final {
                    let field_name = node_text(name_node, source);
                    let visibility = if is_public { "public" } else { "protected" };
                    let start = decl_node.start_position();

                    // Severity graduation by type
                    let type_text = decl_node
                        .child_by_field_name("type")
                        .map(|t| {
                            if t.kind() == "generic_type" {
                                t.named_child(0)
                                    .map(|n| node_text(n, source))
                                    .unwrap_or(node_text(t, source))
                            } else {
                                node_text(t, source)
                            }
                        })
                        .unwrap_or("");
                    let severity = if COLLECTION_TYPES.contains(&type_text) {
                        "error"
                    } else if is_primitive_type(type_text) {
                        "info"
                    } else {
                        "warning"
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "mutable_public_field".to_string(),
                        message: format!(
                            "field `{field_name}` is {visibility} and non-final — use getters/setters or make it final"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MutablePublicFieldsPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Test.java",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_mutable_public_field() {
        let src = "class Foo { public int x; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_public_field");
        assert!(findings[0].message.contains("`x`"));
    }

    #[test]
    fn clean_public_final() {
        let src = "class Foo { public final int x = 1; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_private() {
        let src = "class Foo { private int x; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_entity_class_skipped() {
        let src = r#"@Entity class User { public String name; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_lombok_data_skipped() {
        let src = r#"@Data class Dto { public int x; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_volatile_field() {
        let src = r#"class Foo { public volatile boolean running; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_collection_field_severity() {
        let src = r#"
class Foo {
    public List items;
    public int x;
}
"#;
        let findings = parse_and_check(src);
        let list_f = findings.iter().find(|f| f.message.contains("items")).unwrap();
        let int_f = findings.iter().find(|f| f.message.contains("`x`")).unwrap();
        assert_eq!(list_f.severity, "error");
        assert_eq!(int_f.severity, "info");
    }

    #[test]
    fn test_protected_field() {
        let src = r#"class Foo { protected String name; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("protected"));
    }

    #[test]
    fn test_autowired_field_skipped() {
        let src = r#"class Foo { @Autowired public UserService userService; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
