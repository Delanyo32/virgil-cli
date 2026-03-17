use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_class_decl_query, extract_snippet, find_capture_index, node_text,
};

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

impl Pipeline for GodClassPipeline {
    fn name(&self) -> &str {
        "god_class"
    }

    fn description(&self) -> &str {
        "Detects classes with too many methods (>10), indicating a need to split responsibilities"
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
                let class_name = node_text(name_node, source);
                let method_count = (0..body_node.named_child_count())
                    .filter_map(|i| body_node.named_child(i))
                    .filter(|child| child.kind() == "method_declaration")
                    .count();

                if method_count > METHOD_THRESHOLD {
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "god_class".to_string(),
                        message: format!(
                            "class `{class_name}` has {method_count} methods (threshold: {METHOD_THRESHOLD}) — consider splitting responsibilities"
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
        let pipeline = GodClassPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    fn gen_methods(n: usize) -> String {
        (0..n)
            .map(|i| format!("    void method{i}() {{}}\n"))
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
}
