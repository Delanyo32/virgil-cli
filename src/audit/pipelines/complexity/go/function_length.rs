use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::complexity_helpers::count_function_lines;
use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};
use crate::language::Language;

const LINE_THRESHOLD: usize = 50;
const STATEMENT_THRESHOLD: usize = 20;

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

fn compile_go_function_query() -> Result<Arc<Query>> {
    let lang = Language::Go.tree_sitter_language();
    let query = Query::new(&lang, FUNCTION_QUERY)
        .with_context(|| "failed to compile Go function query")?;
    Ok(Arc::new(query))
}

pub struct FunctionLengthPipeline {
    query: Arc<Query>,
}

impl FunctionLengthPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            query: compile_go_function_query()?,
        })
    }
}

impl Pipeline for FunctionLengthPipeline {
    fn name(&self) -> &str {
        "function_length"
    }

    fn description(&self) -> &str {
        "Detects functions that are too long (>50 lines) or have too many statements (>20)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let fn_name_idx = find_capture_index(&self.query, "fn_name");
        let fn_body_idx = find_capture_index(&self.query, "fn_body");
        let func_idx = find_capture_index(&self.query, "func");

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
                let (lines, stmts) = count_function_lines(body);
                let start = func.start_position();

                if lines > LINE_THRESHOLD {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "function_length".to_string(),
                        pattern: "function_too_long".to_string(),
                        message: format!(
                            "Function '{fn_name}' is {lines} lines long (threshold: {LINE_THRESHOLD})"
                        ),
                        snippet: extract_snippet(source, func, 3),
                    });
                }

                if stmts > STATEMENT_THRESHOLD {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "function_length".to_string(),
                        pattern: "too_many_statements".to_string(),
                        message: format!(
                            "Function '{fn_name}' has {stmts} statements (threshold: {STATEMENT_THRESHOLD})"
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
        let pipeline = FunctionLengthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_long_function() {
        // Generate a function with 55+ lines of variable assignments
        let mut body_lines = String::new();
        for i in 0..55 {
            body_lines.push_str(&format!("    x{i} := {i}\n"));
        }
        let src = format!(
            "package main\n\nfunc longFunc() {{\n{body_lines}}}\n"
        );
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "function_too_long"));
    }

    #[test]
    fn detects_too_many_statements() {
        let mut stmts = String::new();
        for i in 0..22 {
            stmts.push_str(&format!("    x{i} := {i}\n"));
        }
        let src = format!(
            "package main\n\nfunc manyStmts() {{\n{stmts}}}\n"
        );
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "too_many_statements"));
    }

    #[test]
    fn no_finding_for_short_function() {
        let src = r#"package main

func short(x int) int {
    a := x + 1
    return a
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
