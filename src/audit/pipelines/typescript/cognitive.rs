use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{ControlFlowConfig, compute_cognitive};
use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};
use crate::language::Language;

const FUNCTION_QUERY: &str = r#"
[
  (function_declaration
    name: (identifier) @fn_name
    body: (statement_block) @fn_body) @func
  (variable_declarator
    name: (identifier) @fn_name
    value: (arrow_function
      body: (statement_block) @fn_body)) @func
  (method_definition
    name: (property_identifier) @fn_name
    body: (statement_block) @fn_body) @func
]
"#;

fn ts_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_case",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("ternary_expression"),
        comment_kinds: &["comment"],
    }
}

pub struct CognitiveComplexityPipeline {
    query: Arc<Query>,
}

impl CognitiveComplexityPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();
        let query = Query::new(&ts_lang, FUNCTION_QUERY)?;
        Ok(Self {
            query: Arc::new(query),
        })
    }
}

impl Pipeline for CognitiveComplexityPipeline {
    fn name(&self) -> &str {
        "cognitive_complexity"
    }

    fn description(&self) -> &str {
        "Detects functions with high cognitive complexity (> 15)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let fn_name_idx = find_capture_index(&self.query, "fn_name");
        let fn_body_idx = find_capture_index(&self.query, "fn_body");
        let func_idx = find_capture_index(&self.query, "func");

        let config = ts_config();
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
                let cog = compute_cognitive(body, &config, source);

                if cog > 15 {
                    let start = func.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "cognitive_complexity".to_string(),
                        pattern: "high_cognitive_complexity".to_string(),
                        message: format!(
                            "Function `{}` has cognitive complexity of {} (threshold: 15)",
                            fn_name, cog
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
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CognitiveComplexityPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_high_cognitive_complexity() {
        // Deeply nested control flow to exceed cognitive complexity of 15
        let source = r#"
function complex(x: number) {
    if (x > 0) {
        for (let i = 0; i < x; i++) {
            if (i > 1) {
                while (i > 2) {
                    if (i > 3) {
                        if (i > 4) {
                            console.log(i);
                        }
                    }
                }
            }
        }
    }
}
"#;
        let findings = parse_and_check(source);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "high_cognitive_complexity");
        assert_eq!(findings[0].pipeline, "cognitive_complexity");
    }

    #[test]
    fn no_finding_for_simple_function() {
        let source = r#"
function simple(x: number): number {
    if (x > 0) {
        return x;
    }
    return 0;
}
"#;
        let findings = parse_and_check(source);
        assert!(findings.is_empty());
    }
}
