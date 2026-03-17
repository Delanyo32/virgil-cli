use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{compute_cognitive, ControlFlowConfig};
use super::primitives::{
    compile_method_with_body_query, extract_snippet, find_capture_index,
};

const COGNITIVE_THRESHOLD: usize = 15;

fn csharp_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_each_statement",
            "while_statement",
            "do_statement",
            "switch_section",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_each_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

pub struct CognitiveComplexityPipeline {
    query: Arc<Query>,
}

impl CognitiveComplexityPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            query: compile_method_with_body_query()?,
        })
    }
}

impl Pipeline for CognitiveComplexityPipeline {
    fn name(&self) -> &str {
        "cognitive_complexity"
    }

    fn description(&self) -> &str {
        "Detects methods with high cognitive complexity (>15)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.query, "method_name");
        let body_idx = find_capture_index(&self.query, "method_body");
        let method_idx = find_capture_index(&self.query, "method");

        let cfg = csharp_config();

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(method_node)) =
                (name_node, body_node, method_node)
            {
                let name = name_node.utf8_text(source).unwrap_or("<unknown>");
                let score = compute_cognitive(body_node, &cfg, source);

                if score > COGNITIVE_THRESHOLD {
                    let start = method_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "cognitive_complexity".to_string(),
                        pattern: "high_cognitive_complexity".to_string(),
                        message: format!(
                            "Cognitive complexity of {score} (threshold: {COGNITIVE_THRESHOLD}) in method '{name}'"
                        ),
                        snippet: extract_snippet(source, method_node, 3),
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CognitiveComplexityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_high_cognitive_complexity() {
        // Deeply nested control flow to exceed threshold of 15
        let src = r#"
class Foo {
    void Complex(int x) {
        if (x > 0) {
            for (int i = 0; i < 10; i++) {
                if (i > 5) {
                    while (x > 0) {
                        if (x == 1) {
                            if (x == 2) {
                            }
                        }
                    }
                }
            }
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "high_cognitive_complexity");
        assert!(findings[0].message.contains("threshold: 15"));
    }

    #[test]
    fn clean_simple_method() {
        let src = r#"
class Foo {
    void Simple() {
        int x = 1;
        if (x > 0) { x = 2; }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
