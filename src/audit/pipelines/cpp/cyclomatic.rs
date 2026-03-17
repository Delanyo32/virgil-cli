use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{compute_cyclomatic, ControlFlowConfig};
use super::primitives::{extract_snippet, find_capture_index};
use crate::language::Language;

const CC_THRESHOLD: usize = 10;

const FUNCTION_QUERY: &str = r#"
(function_definition
  declarator: (function_declarator
    declarator: (_) @fn_name)
  body: (compound_statement) @fn_body) @func
"#;

fn cpp_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_range_loop",
            "while_statement",
            "do_statement",
            "case_statement",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_range_loop",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
            "lambda_expression",
        ],
        flat_increments: &["else_clause", "goto_statement"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
        comment_kinds: &["comment"],
    }
}

fn extract_function_name(name_node: tree_sitter::Node, source: &[u8]) -> String {
    if name_node.kind() == "identifier" {
        return name_node.utf8_text(source).unwrap_or("").to_string();
    }
    if name_node.kind() == "qualified_identifier" || name_node.kind() == "field_identifier" {
        return name_node.utf8_text(source).unwrap_or("").to_string();
    }
    let mut cursor = name_node.walk();
    for child in name_node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "field_identifier" {
            return child.utf8_text(source).unwrap_or("").to_string();
        }
    }
    name_node.utf8_text(source).unwrap_or("<unknown>").to_string()
}

pub struct CyclomaticComplexityPipeline {
    query: Arc<Query>,
}

impl CyclomaticComplexityPipeline {
    pub fn new() -> Result<Self> {
        let ts_lang = Language::Cpp.tree_sitter_language();
        let query = Query::new(&ts_lang, FUNCTION_QUERY)?;
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
        "Detects functions with high cyclomatic complexity (>10)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.query, "fn_name");
        let body_idx = find_capture_index(&self.query, "fn_body");
        let func_idx = find_capture_index(&self.query, "func");

        let cfg = cpp_config();

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
            let func_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == func_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(func_node)) =
                (name_node, body_node, func_node)
            {
                let name = extract_function_name(name_node, source);
                let cc = compute_cyclomatic(body_node, &cfg, source);

                if cc > CC_THRESHOLD {
                    let start = func_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "cyclomatic_complexity".to_string(),
                        pattern: "high_cyclomatic_complexity".to_string(),
                        message: format!(
                            "Cyclomatic complexity of {cc} (threshold: {CC_THRESHOLD}) in function '{name}'"
                        ),
                        snippet: extract_snippet(source, func_node, 3),
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CyclomaticComplexityPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_high_cyclomatic_complexity() {
        // Function with 11 if statements: CC = 1 + 11 = 12 > 10
        let src = r#"
void complex(int x) {
    if (x == 1) {}
    if (x == 2) {}
    if (x == 3) {}
    if (x == 4) {}
    if (x == 5) {}
    if (x == 6) {}
    if (x == 7) {}
    if (x == 8) {}
    if (x == 9) {}
    if (x == 10) {}
    if (x == 11) {}
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "high_cyclomatic_complexity");
        assert!(findings[0].message.contains("threshold: 10"));
    }

    #[test]
    fn clean_simple_function() {
        let src = r#"
void simple(int x) {
    if (x > 0) { x = 2; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
