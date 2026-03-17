use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_function_lines;
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

pub struct FunctionLengthPipeline {
    query: Arc<Query>,
}

impl FunctionLengthPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();
        let query = Query::new(&ts_lang, FUNCTION_QUERY)?;
        Ok(Self {
            query: Arc::new(query),
        })
    }
}

impl Pipeline for FunctionLengthPipeline {
    fn name(&self) -> &str {
        "function_length"
    }

    fn description(&self) -> &str {
        "Detects functions that are too long (> 50 lines or > 20 statements)"
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

                if lines > 50 {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "function_length".to_string(),
                        pattern: "function_too_long".to_string(),
                        message: format!(
                            "Function `{}` is {} lines long (threshold: 50)",
                            fn_name, lines
                        ),
                        snippet: extract_snippet(source, func, 3),
                    });
                }

                if stmts > 20 {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "function_length".to_string(),
                        pattern: "too_many_statements".to_string(),
                        message: format!(
                            "Function `{}` has {} statements (threshold: 20)",
                            fn_name, stmts
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
        let pipeline = FunctionLengthPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_long_function() {
        // Generate a function with > 50 lines
        let mut lines = vec!["function longFunc() {".to_string()];
        for i in 0..52 {
            lines.push(format!("    let x{} = {};", i, i));
        }
        lines.push("}".to_string());
        let source = lines.join("\n");
        let findings = parse_and_check(&source);
        assert!(!findings.is_empty());
        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"function_too_long") || patterns.contains(&"too_many_statements"));
    }

    #[test]
    fn no_finding_for_short_function() {
        let source = r#"
function short(x: number): number {
    const a = x + 1;
    return a;
}
"#;
        let findings = parse_and_check(source);
        assert!(findings.is_empty());
    }
}
