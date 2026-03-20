use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{ControlFlowConfig, compute_cyclomatic};
use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};
use crate::language::Language;

const FUNCTION_QUERY: &str = r#"
(function_definition
  name: (identifier) @fn_name
  body: (block) @fn_body) @func
"#;

fn py_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "elif_clause",
            "for_statement",
            "while_statement",
            "except_clause",
            "with_statement",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "while_statement",
            "except_clause",
            "with_statement",
        ],
        flat_increments: &["elif_clause", "else_clause"],
        logical_operators: &["and", "or"],
        binary_expression_kind: "boolean_operator",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

pub struct CyclomaticComplexityPipeline {
    query: Arc<Query>,
}

impl CyclomaticComplexityPipeline {
    pub fn new() -> Result<Self> {
        let py_lang = Language::Python.tree_sitter_language();
        let query = Query::new(&py_lang, FUNCTION_QUERY)?;
        Ok(Self {
            query: Arc::new(query),
        })
    }
}

impl Pipeline for CyclomaticComplexityPipeline {
    fn name(&self) -> &str {
        "cyclomatic_complexity"
    }

    fn description(&self) -> &str {
        "Detects functions with high cyclomatic complexity (CC > 10)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let fn_name_idx = find_capture_index(&self.query, "fn_name");
        let fn_body_idx = find_capture_index(&self.query, "fn_body");
        let func_idx = find_capture_index(&self.query, "func");

        let config = py_config();
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
                let cc = compute_cyclomatic(body, &config, source);

                if cc > 10 {
                    let start = func.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "cyclomatic_complexity".to_string(),
                        pattern: "high_cyclomatic_complexity".to_string(),
                        message: format!(
                            "Function `{}` has cyclomatic complexity of {} (threshold: 10)",
                            fn_name, cc
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CyclomaticComplexityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_high_cyclomatic_complexity() {
        // CC = 1 + 11 if statements = 12 > 10
        let source = r#"def complex(x):
    if x > 1:
        pass
    if x > 2:
        pass
    if x > 3:
        pass
    if x > 4:
        pass
    if x > 5:
        pass
    if x > 6:
        pass
    if x > 7:
        pass
    if x > 8:
        pass
    if x > 9:
        pass
    if x > 10:
        pass
    if x > 11:
        pass
"#;
        let findings = parse_and_check(source);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "high_cyclomatic_complexity");
        assert_eq!(findings[0].pipeline, "cyclomatic_complexity");
    }

    #[test]
    fn no_finding_for_simple_function() {
        let source = r#"def simple(x):
    if x > 0:
        return x
    return 0
"#;
        let findings = parse_and_check(source);
        assert!(findings.is_empty());
    }
}
