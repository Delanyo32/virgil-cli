use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{compute_cognitive, ControlFlowConfig};
use super::primitives;
use crate::audit::primitives::{extract_snippet, find_capture_index};

const COGNITIVE_THRESHOLD: usize = 15;

fn config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_expression",
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_arm",
        ],
        nesting_increments: &[
            "if_expression",
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_expression",
            "closure_expression",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: None,
        comment_kinds: &["line_comment", "block_comment"],
    }
}

pub struct CognitiveComplexityPipeline {
    query: Arc<Query>,
}

impl CognitiveComplexityPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            query: primitives::compile_function_item_query()?,
        })
    }
}

impl Pipeline for CognitiveComplexityPipeline {
    fn name(&self) -> &str {
        "cognitive_complexity"
    }

    fn description(&self) -> &str {
        "Detects functions with high cognitive complexity (>15)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.query, "fn_name");
        let body_idx = find_capture_index(&self.query, "fn_body");
        let def_idx = find_capture_index(&self.query, "fn_def");

        let cfg = config();

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
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == def_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(def_node)) =
                (name_node, body_node, def_node)
            {
                let name = name_node.utf8_text(source).unwrap_or("<unknown>");
                let score = compute_cognitive(body_node, &cfg, source);

                if score > COGNITIVE_THRESHOLD {
                    let start = def_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "cognitive_complexity".to_string(),
                        pattern: "high_cognitive_complexity".to_string(),
                        message: format!(
                            "Cognitive complexity of {score} (threshold: {COGNITIVE_THRESHOLD}) in function '{name}'"
                        ),
                        snippet: extract_snippet(source, def_node, 3),
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CognitiveComplexityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_high_cognitive_complexity() {
        // Deeply nested control flow to exceed threshold of 15
        // if: +1(nesting=0), for: +1+1=2(nesting=1), match: nesting_increment(nesting=2),
        //   match_arm: counted as decision_point but not nesting_increment
        // if inside for: +1+2=3(nesting=2), while: +1+3=4(nesting=3),
        // if inside while: +1+4=5(nesting=4), loop: +1+5=6(nesting=5)
        let src = r#"
fn complex(x: i32) {
    if x > 0 {
        for i in 0..10 {
            if i > 5 {
                while x > 0 {
                    if x == 1 {
                        loop {
                            if x == 2 {
                                break;
                            }
                            break;
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
    fn clean_simple_function() {
        let src = r#"
fn simple(x: i32) -> i32 {
    let y = x + 1;
    if y > 0 { y } else { 0 }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
