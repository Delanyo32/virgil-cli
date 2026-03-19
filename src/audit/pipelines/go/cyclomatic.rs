use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{ControlFlowConfig, compute_cyclomatic};
use crate::language::Language;

const CC_THRESHOLD: usize = 10;

const FUNCTION_QUERY: &str = r#"
[
  (function_declaration
    name: (identifier) @fn_name
    body: (block) @fn_body) @func
  (method_declaration
    name: (field_identifier) @fn_name
    body: (block) @fn_body) @func
]
"#;

fn config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "expression_case",
            "type_case",
            "communication_case",
            "default_case",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "expression_switch_statement",
            "type_switch_statement",
            "select_statement",
            "func_literal",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: None,
        comment_kinds: &["comment"],
    }
}

fn compile_go_function_query() -> Result<Arc<Query>> {
    let lang = Language::Go.tree_sitter_language();
    let query =
        Query::new(&lang, FUNCTION_QUERY).with_context(|| "failed to compile Go function query")?;
    Ok(Arc::new(query))
}

pub struct CyclomaticComplexityPipeline {
    query: Arc<Query>,
}

impl CyclomaticComplexityPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            query: compile_go_function_query()?,
        })
    }
}

impl Pipeline for CyclomaticComplexityPipeline {
    fn name(&self) -> &str {
        "cyclomatic_complexity"
    }

    fn description(&self) -> &str {
        "Detects functions with high cyclomatic complexity (>10)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let fn_name_idx = find_capture_index(&self.query, "fn_name");
        let fn_body_idx = find_capture_index(&self.query, "fn_body");
        let func_idx = find_capture_index(&self.query, "func");

        let cfg = config();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let mut name_node = None;
            let mut body_node = None;
            let mut func_node = None;

            for cap in m.captures {
                if cap.index as usize == fn_name_idx {
                    name_node = Some(cap.node);
                } else if cap.index as usize == fn_body_idx {
                    body_node = Some(cap.node);
                } else if cap.index as usize == func_idx {
                    func_node = Some(cap.node);
                }
            }

            if let (Some(name), Some(body), Some(func)) = (name_node, body_node, func_node) {
                let fn_name = node_text(name, source);
                let cc = compute_cyclomatic(body, &cfg, source);

                if cc > CC_THRESHOLD {
                    let start = func.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "cyclomatic_complexity".to_string(),
                        pattern: "high_cyclomatic_complexity".to_string(),
                        message: format!(
                            "Cyclomatic complexity of {cc} (threshold: {CC_THRESHOLD}) in function '{fn_name}'"
                        ),
                        snippet: extract_snippet(source, func, 3),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CyclomaticComplexityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_high_cyclomatic_complexity() {
        // CC = 1 base + 11 if statements + 2 logical operators = 14 > 10
        let src = r#"package main

func complex(x int) {
    if x == 1 {
    }
    if x == 2 {
    }
    if x == 3 {
    }
    if x == 4 {
    }
    if x == 5 {
    }
    if x == 6 {
    }
    if x == 7 {
    }
    if x == 8 {
    }
    if x == 9 {
    }
    if x == 10 {
    }
    if x == 11 && x > 0 || x < 100 {
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "high_cyclomatic_complexity");
        assert_eq!(findings[0].pipeline, "cyclomatic_complexity");
        assert!(findings[0].message.contains("threshold: 10"));
    }

    #[test]
    fn no_finding_for_simple_function() {
        let src = r#"package main

func simple(x int) int {
    if x > 0 {
        return x
    }
    return 0
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
