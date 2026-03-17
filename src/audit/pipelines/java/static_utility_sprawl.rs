use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::java_primitives::{
    compile_class_decl_query, extract_snippet, find_capture_index, has_modifier, node_text,
};

const STATIC_METHOD_THRESHOLD: usize = 3;

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

impl Pipeline for StaticUtilitySprawlPipeline {
    fn name(&self) -> &str {
        "static_utility_sprawl"
    }

    fn description(&self) -> &str {
        "Detects utility classes with many static methods and no instance methods — consider splitting"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "static_utility_class".to_string(),
                        message: format!(
                            "class `{class_name}` has {static_count} static methods and no instance methods — consider splitting into focused utility classes"
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
        let pipeline = StaticUtilitySprawlPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    fn gen_static_methods(n: usize) -> String {
        (0..n)
            .map(|i| format!("    static void method{i}() {{}}\n"))
            .collect()
    }

    #[test]
    fn detects_static_utility_class() {
        let methods = gen_static_methods(5);
        let src = format!("class Utils {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "static_utility_class");
        assert!(findings[0].message.contains("5 static methods"));
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
    void instanceMethod() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_below_threshold() {
        let methods = gen_static_methods(2);
        let src = format!("class Small {{\n{methods}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }
}
