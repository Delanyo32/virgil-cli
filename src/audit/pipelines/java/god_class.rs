use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{compile_class_decl_query, extract_snippet, find_capture_index, node_text};
use crate::audit::pipelines::helpers::{has_annotation, has_suppress_warnings};

const METHOD_THRESHOLD: usize = 10;

pub struct GodClassPipeline {
    class_query: Arc<Query>,
}

impl GodClassPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_decl_query()?,
        })
    }
}

impl GraphPipeline for GodClassPipeline {
    fn name(&self) -> &str {
        "god_class"
    }

    fn description(&self) -> &str {
        "Detects classes with too many methods (>10), indicating a need to split responsibilities"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let class_name_idx = find_capture_index(&self.class_query, "class_name");
        let class_body_idx = find_capture_index(&self.class_query, "class_body");
        let class_decl_idx = find_capture_index(&self.class_query, "class_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_body_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(decl_node)) =
                (name_node, body_node, decl_node)
            {
                // Skip enum, interface, and record declarations
                let kind = decl_node.kind();
                if kind == "enum_declaration"
                    || kind == "interface_declaration"
                    || kind == "record_declaration"
                {
                    continue;
                }

                let class_name = node_text(name_node, source);

                // Skip classes with Lombok annotations that generate methods
                if has_annotation(decl_node, source, "Data")
                    || has_annotation(decl_node, source, "Getter")
                    || has_annotation(decl_node, source, "Setter")
                    || has_annotation(decl_node, source, "Builder")
                {
                    continue;
                }

                // Skip suppressed classes (@SuppressWarnings("god-class") or @Generated)
                if has_suppress_warnings(decl_node, source, "god-class") {
                    continue;
                }

                // Skip classes whose name ends with "Builder"
                if class_name.ends_with("Builder") {
                    continue;
                }

                // Count only non-accessor methods:
                // Skip methods matching get*/set*/is* with <= 2 statements and <= 3 lines
                let method_count = (0..body_node.named_child_count())
                    .filter_map(|i| body_node.named_child(i))
                    .filter(|child| child.kind() == "method_declaration")
                    .filter(|child| !is_accessor_method(*child, source))
                    .count();

                // Count field declarations for composite score
                let field_count = (0..body_node.named_child_count())
                    .filter_map(|i| body_node.named_child(i))
                    .filter(|child| child.kind() == "field_declaration")
                    .count();

                // Composite score: methods + fields/2
                let composite = method_count + field_count / 2;

                // Framework annotation awareness: Configuration, Component, RestController
                // These classes legitimately have many methods; use higher threshold
                if has_annotation(decl_node, source, "Configuration")
                    || has_annotation(decl_node, source, "Component")
                    || has_annotation(decl_node, source, "RestController")
                {
                    if composite > METHOD_THRESHOLD * 2 {
                        let start = decl_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "god_class".to_string(),
                            message: format!(
                                "framework class `{class_name}` has {method_count} methods and {field_count} fields (composite {composite}, threshold: {}) — consider splitting responsibilities",
                                METHOD_THRESHOLD * 2
                            ),
                            snippet: extract_snippet(source, decl_node, 3),
                        });
                    }
                    continue;
                }

                if composite > METHOD_THRESHOLD {
                    let severity = if method_count >= 31 {
                        "error"
                    } else if method_count >= 21 {
                        "warning"
                    } else {
                        "info"
                    };

                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "god_class".to_string(),
                        message: format!(
                            "class `{class_name}` has {method_count} methods and {field_count} fields (composite {composite}, threshold: {METHOD_THRESHOLD}) — consider splitting responsibilities"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
                    });
                }
            }
        }

        findings
    }
}

/// Check if a method is a simple accessor (getter/setter with small body).
/// Matches methods named get*, set*, is* that have <= 2 statements and <= 3 lines.
fn is_accessor_method(method_node: tree_sitter::Node, source: &[u8]) -> bool {
    let name = method_node
        .child_by_field_name("name")
        .map(|n| node_text(n, source))
        .unwrap_or("");

    let is_accessor_name =
        name.starts_with("get") || name.starts_with("set") || name.starts_with("is");

    if !is_accessor_name {
        return false;
    }

    if let Some(body) = method_node.child_by_field_name("body") {
        let stmt_count = body.named_child_count();
        let line_count = body.end_position().row - body.start_position().row;
        return stmt_count <= 2 && line_count <= 3;
    }

    false
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
        let pipeline = GodClassPipeline::new().unwrap();
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

    fn gen_methods(n: usize) -> String {
        (0..n)
            .map(|i| format!("    void method{i}() {{}}\n"))
            .collect()
    }

    fn gen_fields(n: usize) -> String {
        (0..n)
            .map(|i| format!("    private int field{i};\n"))
            .collect()
    }

    #[test]
    fn detects_god_class() {
        let methods = gen_methods(12);
        let src = format!("class BigClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "god_class");
        assert!(findings[0].message.contains("12 methods"));
    }

    #[test]
    fn clean_small_class() {
        let methods = gen_methods(3);
        let src = format!("class SmallClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn exactly_at_threshold_is_clean() {
        let methods = gen_methods(10);
        let src = format!("class EdgeClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_severity_graduation() {
        let methods = gen_methods(25);
        let src = format!("class HugeClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn test_severity_error_for_very_large() {
        let methods = gen_methods(35);
        let src = format!("class MassiveClass {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn test_generated_annotation() {
        let methods = gen_methods(15);
        let src = format!("@Generated class Gen {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_interface_default_methods() {
        let src = r#"
interface Foo {
    default void m1() {}
    default void m2() {}
    default void m3() {}
    default void m4() {}
    default void m5() {}
    default void m6() {}
    default void m7() {}
    default void m8() {}
    default void m9() {}
    default void m10() {}
    default void m11() {}
    default void m12() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_fields_contribute_to_composite_score() {
        // 9 methods + 4 fields = composite 11 (> threshold 10)
        let methods = gen_methods(9);
        let fields = gen_fields(4);
        let src = format!("class FieldHeavy {{\n{fields}{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("9 methods"));
        assert!(findings[0].message.contains("4 fields"));
    }

    #[test]
    fn test_framework_annotation_higher_threshold() {
        // 15 methods with @RestController: should NOT be flagged (below 2x threshold)
        let methods = gen_methods(15);
        let src = format!("@RestController class Api {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_framework_annotation_flagged_when_excessive() {
        // 25 methods with @RestController: composite > 20, should be flagged as info
        let methods = gen_methods(25);
        let src = format!("@RestController class Api {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn test_accessor_with_many_lines_not_filtered() {
        // A method named getData with a large body should NOT be treated as an accessor
        let src = r#"
class Svc {
    void method1() {}
    void method2() {}
    void method3() {}
    void method4() {}
    void method5() {}
    void method6() {}
    void method7() {}
    void method8() {}
    void method9() {}
    void method10() {}
    String getData() {
        String a = "x";
        String b = "y";
        String c = "z";
        String d = "w";
        String e = "v";
        return a + b + c + d + e;
    }
}
"#;
        let findings = parse_and_check(src);
        // getData has > 3 lines, so it's NOT filtered as accessor; 11 methods total
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("11 methods"));
    }

    #[test]
    fn test_suppress_warnings_annotation() {
        let methods = gen_methods(15);
        let src = format!(
            "@SuppressWarnings(\"god-class\") class Suppressed {{\n{methods}}}\n"
        );
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_lombok_data_skipped() {
        let methods = gen_methods(15);
        let src = format!("@Data class Dto {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_builder_name_skipped() {
        let methods = gen_methods(15);
        let src = format!("class UserBuilder {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }
}
