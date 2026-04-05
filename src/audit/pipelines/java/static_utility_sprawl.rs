use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{has_annotation, has_suppress_warnings};

use super::primitives::{
    compile_class_decl_query, extract_snippet, find_capture_index, has_modifier, node_text,
};

const STATIC_METHOD_THRESHOLD: usize = 8;

const UTILITY_SUFFIXES: &[&str] = &["Utils", "Helper", "Constants", "Util"];

pub struct StaticUtilitySprawlPipeline {
    class_query: Arc<Query>,
}

impl StaticUtilitySprawlPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_decl_query()?,
        })
    }
}

fn has_private_constructor(body_node: tree_sitter::Node, source: &[u8]) -> bool {
    for i in 0..body_node.named_child_count() {
        if let Some(child) = body_node.named_child(i)
            && child.kind() == "constructor_declaration"
            && has_modifier(child, source, "private")
        {
            return true;
        }
    }
    false
}

impl GraphPipeline for StaticUtilitySprawlPipeline {
    fn name(&self) -> &str {
        "static_utility_sprawl"
    }

    fn description(&self) -> &str {
        "Detects utility classes with many static methods and no instance methods — consider splitting"
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
                // Skip @SuppressWarnings("static-utility")
                if has_suppress_warnings(decl_node, source, "static-utility") {
                    continue;
                }

                // Skip @UtilityClass annotation
                if has_annotation(decl_node, source, "UtilityClass") {
                    continue;
                }

                // Skip abstract classes
                if has_modifier(decl_node, source, "abstract") {
                    continue;
                }

                // Skip classes with private constructor (intentional utility pattern)
                if has_private_constructor(body_node, source) {
                    continue;
                }

                let methods: Vec<_> = (0..body_node.named_child_count())
                    .filter_map(|i| body_node.named_child(i))
                    .filter(|child| child.kind() == "method_declaration")
                    .collect();

                let total = methods.len();
                if total == 0 {
                    continue;
                }

                let static_count = methods
                    .iter()
                    .filter(|m| has_modifier(**m, source, "static"))
                    .count();

                // Flag if all methods are static and count exceeds threshold
                if static_count == total && total > STATIC_METHOD_THRESHOLD {
                    let class_name = node_text(name_node, source);
                    let start = decl_node.start_position();

                    // Severity graduation: 8-15 → info, 16+ → warning
                    let severity = if static_count >= 16 {
                        "warning"
                    } else {
                        "info"
                    };

                    // Check if the class name follows utility naming conventions
                    let is_utility_name =
                        UTILITY_SUFFIXES.iter().any(|s| class_name.ends_with(s));

                    let message = if is_utility_name {
                        format!(
                            "class `{class_name}` has {static_count} static methods and no instance methods (recognized utility class naming convention) — consider splitting into focused utility classes"
                        )
                    } else {
                        format!(
                            "class `{class_name}` has {static_count} static methods and no instance methods — consider splitting into focused utility classes"
                        )
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "static_utility_class".to_string(),
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
        let pipeline = StaticUtilitySprawlPipeline::new().unwrap();
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

    fn gen_static_methods(n: usize) -> String {
        (0..n)
            .map(|i| format!("    static void method{i}() {{}}\n"))
            .collect()
    }

    #[test]
    fn detects_static_utility_class() {
        let methods = gen_static_methods(10);
        let src = format!("class Utils {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "static_utility_class");
        assert!(findings[0].message.contains("10 static methods"));
    }

    #[test]
    fn clean_mixed_methods() {
        let src = r#"
class Utils {
    static void a() {}
    static void b() {}
    static void c() {}
    static void d() {}
    static void e() {}
    static void f() {}
    static void g() {}
    static void h() {}
    static void i() {}
    static void j() {}
    void instanceMethod() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_below_threshold() {
        let methods = gen_static_methods(7);
        let src = format!("class Small {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_private_constructor_skipped() {
        let src = r#"
class Utils {
    private Utils() {}
    static void a() {}
    static void b() {}
    static void c() {}
    static void d() {}
    static void e() {}
    static void f() {}
    static void g() {}
    static void h() {}
    static void i() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_utility_class_annotation() {
        let methods = gen_static_methods(10);
        let src = format!("@UtilityClass class Helpers {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_abstract_class_skipped() {
        let methods = gen_static_methods(10);
        let src = format!("abstract class Base {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_severity_graduation() {
        let methods = gen_static_methods(20);
        let src = format!("class BigUtils {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn test_utils_naming_convention() {
        let methods = gen_static_methods(10);
        let src = format!("class StringUtils {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("utility"));
    }
}
