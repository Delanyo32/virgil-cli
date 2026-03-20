use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_call_query, extract_snippet, find_capture_index, node_text,
};

pub struct SqlInjectionPipeline {
    method_query: Arc<Query>,
}

impl SqlInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_call_query()?,
        })
    }
}

impl Pipeline for SqlInjectionPipeline {
    fn name(&self) -> &str {
        "sql_injection"
    }

    fn description(&self) -> &str {
        "Detects SQL injection risks: string interpolation/concatenation in SQL queries"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.method_query, "method_name");
        let call_idx = find_capture_index(&self.method_query, "call");

        let sql_methods = ["Query", "QueryRow", "Exec", "QueryContext", "ExecContext"];

        while let Some(m) = matches.next() {
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(method_node), Some(call_node)) = (method_node, call_node) {
                let method_name = node_text(method_node, source);

                if !sql_methods.contains(&method_name) {
                    continue;
                }

                let call_text = node_text(call_node, source);

                if call_text.contains("fmt.Sprintf") {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_interpolation".to_string(),
                        message: format!(
                            "SQL query via .{method_name}() uses fmt.Sprintf — use parameterized queries instead"
                        ),
                        snippet: extract_snippet(source, call_node, 1),
                    });
                } else if call_text.contains('+') {
                    // Check for string concatenation in the arguments
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_concat".to_string(),
                        message: format!(
                            "SQL query via .{method_name}() uses string concatenation — use parameterized queries instead"
                        ),
                        snippet: extract_snippet(source, call_node, 1),
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SqlInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_sprintf_in_query() {
        let src = r#"package main

func getUser(db DB, id string) {
	db.Query(fmt.Sprintf("SELECT * FROM users WHERE id = %s", id))
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_string_interpolation");
    }

    #[test]
    fn detects_concat_in_query() {
        let src = r#"package main

func getUser(db DB, id string) {
	db.Query("SELECT * FROM users WHERE id = " + id)
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_string_concat");
    }

    #[test]
    fn ignores_parameterized_query() {
        let src = r#"package main

func getUser(db DB, id string) {
	db.Query("SELECT * FROM users WHERE id = $1", id)
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_findings() {
        let src = r#"package main

import "fmt"

func main() {
	fmt.Println("hello")
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
