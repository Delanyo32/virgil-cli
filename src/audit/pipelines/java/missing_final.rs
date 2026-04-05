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

pub struct MissingFinalPipeline {
    field_query: Arc<Query>,
}

impl MissingFinalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_decl_query()?,
        })
    }
}

fn enclosing_class(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut p = node.parent();
    while let Some(n) = p {
        if n.kind() == "class_declaration" {
            return Some(n);
        }
        p = n.parent();
    }
    None
}

fn check_assignments_recursive(
    node: tree_sitter::Node,
    field_name: &str,
    source: &[u8],
    in_constructor: bool,
    assigned_in_ctor: &mut bool,
    assigned_outside_ctor: &mut bool,
) {
    // Check for assignment to this.field_name or just field_name = ...
    if node.kind() == "assignment_expression"
        && let Some(lhs) = node.child_by_field_name("left")
    {
        let lhs_text = node_text(lhs, source);
        if lhs_text == field_name || lhs_text == format!("this.{field_name}") {
            if in_constructor {
                *assigned_in_ctor = true;
            } else {
                *assigned_outside_ctor = true;
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_in_ctor = in_constructor || child.kind() == "constructor_declaration";
        // Don't recurse into nested classes
        if child.kind() == "class_declaration" {
            continue;
        }
        check_assignments_recursive(
            child,
            field_name,
            source,
            child_in_ctor,
            assigned_in_ctor,
            assigned_outside_ctor,
        );
    }
}

/// Checks whether a field is only assigned in constructors.
/// Returns:
///   Some(true)  — only assigned in constructor(s) (should be final)
///   Some(false) — never assigned (potentially unused or set via reflection)
///   None        — assigned in methods too (not a missing-final issue)
fn is_assigned_only_in_constructors(
    field_name: &str,
    class_body: tree_sitter::Node,
    source: &[u8],
) -> Option<bool> {
    let mut assigned_in_constructor = false;
    let mut assigned_outside_constructor = false;

    check_assignments_recursive(
        class_body,
        field_name,
        source,
        false, // not initially in constructor
        &mut assigned_in_constructor,
        &mut assigned_outside_constructor,
    );

    if assigned_in_constructor && !assigned_outside_constructor {
        Some(true) // Only assigned in constructor(s)
    } else if !assigned_in_constructor && !assigned_outside_constructor {
        Some(false) // Never assigned (could be unused or set via reflection)
    } else {
        None // Assigned in methods too -- don't flag
    }
}

impl GraphPipeline for MissingFinalPipeline {
    fn name(&self) -> &str {
        "missing_final"
    }

    fn description(&self) -> &str {
        "Detects private fields that are not final — consider making them final for immutability"
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
                let is_private = has_modifier(decl_node, source, "private");
                let is_final = has_modifier(decl_node, source, "final");

                if is_private && !is_final {
                    // Skip @SuppressWarnings("missing-final")
                    if has_suppress_warnings(decl_node, source, "missing-final") {
                        continue;
                    }

                    // Skip injection-annotated fields
                    if has_annotation(decl_node, source, "Autowired")
                        || has_annotation(decl_node, source, "Inject")
                        || has_annotation(decl_node, source, "Value")
                        || has_annotation(decl_node, source, "Setter")
                    {
                        continue;
                    }

                    // Skip volatile fields
                    let is_volatile = has_modifier(decl_node, source, "volatile");
                    if is_volatile {
                        continue;
                    }

                    // Skip @Entity class fields
                    if let Some(class) = enclosing_class(decl_node)
                        && has_annotation(class, source, "Entity")
                    {
                        continue;
                    }

                    let field_name = node_text(name_node, source);
                    let start = decl_node.start_position();

                    // Determine severity based on constructor-only assignment
                    let class_body = enclosing_class(decl_node)
                        .and_then(|c| c.child_by_field_name("body"));

                    let (severity, message) = if let Some(body) = class_body {
                        match is_assigned_only_in_constructors(field_name, body, source) {
                            Some(true) => (
                                "warning".to_string(),
                                format!(
                                    "private field `{field_name}` should be final — it is only assigned in the constructor"
                                ),
                            ),
                            Some(false) => (
                                "info".to_string(),
                                format!(
                                    "private field `{field_name}` is potentially unused or set via reflection — consider making it final"
                                ),
                            ),
                            None => {
                                // Assigned in methods too -- not a missing-final issue
                                continue;
                            }
                        }
                    } else {
                        // No class body found (shouldn't happen), fall back to info
                        (
                            "info".to_string(),
                            format!(
                                "private field `{field_name}` is not final — consider making it final for immutability"
                            ),
                        )
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity,
                        pipeline: self.name().to_string(),
                        pattern: "missing_final_field".to_string(),
                        message,
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingFinalPipeline::new().unwrap();
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
    fn detects_missing_final() {
        let src = "class Foo { private String name; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_final_field");
        assert!(findings[0].message.contains("`name`"));
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn clean_private_final() {
        let src = "class Foo { private final String name = \"x\"; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_public_field() {
        // Public non-final is handled by mutable_public_fields, not this pipeline
        let src = "class Foo { public String name; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_autowired_field_skipped() {
        let src = r#"class Foo { @Autowired private UserService userService; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_volatile_field_skipped() {
        let src = r#"class Foo { private volatile boolean running; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_constructor_only_assignment() {
        let src = r#"
class Foo {
    private String name;
    Foo(String name) {
        this.name = name;
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
        assert!(findings[0].message.contains("should be final"));
    }

    #[test]
    fn test_setter_assigned_field() {
        let src = r#"
class Foo {
    private String name;
    void setName(String n) { this.name = n; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_entity_class() {
        let src = r#"@Entity class User { private String name; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_lombok_setter() {
        let src = r#"class Foo { @Setter private String name; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
