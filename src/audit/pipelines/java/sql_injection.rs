use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_invocation_with_object_query, extract_snippet, find_capture_index, node_text,
};

const SQL_METHODS: &[&str] = &["executeQuery", "executeUpdate", "execute"];

pub struct SqlInjectionPipeline {
    method_query: Arc<Query>,
}

impl SqlInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_invocation_with_object_query()?,
        })
    }
}

impl Pipeline for SqlInjectionPipeline {
    fn name(&self) -> &str {
        "sql_injection"
    }

    fn description(&self) -> &str {
        "Detects SQL injection risks: string concatenation or String.format in SQL query methods"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.method_query, "method_name");
        let args_idx = find_capture_index(&self.method_query, "args");
        let invocation_idx = find_capture_index(&self.method_query, "invocation");

        while let Some(m) = matches.next() {
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == invocation_idx)
                .map(|c| c.node);

            if let (Some(method_node), Some(args_node), Some(inv_node)) =
                (method_node, args_node, inv_node)
            {
                let method_name = node_text(method_node, source);
                if !SQL_METHODS.contains(&method_name) {
                    continue;
                }

                let args_text = node_text(args_node, source);

                if args_text.contains("String.format") || args_text.contains("string.format") {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_format".to_string(),
                        message: format!(
                            "`.{method_name}()` uses String.format — use PreparedStatement with ? placeholders"
                        ),
                        snippet: extract_snippet(source, inv_node, 2),
                    });
                } else if args_text.contains('+') {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_concat".to_string(),
                        message: format!(
                            "`.{method_name}()` uses string concatenation — use PreparedStatement with ? placeholders"
                        ),
                        snippet: extract_snippet(source, inv_node, 2),
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
        let pipeline = SqlInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_string_concat_in_execute_query() {
        let src = r#"class Dao {
    void find(String id) {
        stmt.executeQuery("SELECT * FROM users WHERE id = " + id);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_string_concat");
    }

    #[test]
    fn detects_string_format_in_execute() {
        let src = r#"class Dao {
    void find(String id) {
        stmt.execute(String.format("SELECT * FROM users WHERE id = %s", id));
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_string_format");
    }

    #[test]
    fn ignores_prepared_statement() {
        let src = r#"class Dao {
    void find(String id) {
        stmt.executeQuery("SELECT * FROM users WHERE id = ?");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_sql() {
        let src = r#"class Foo {
    void bar() {
        System.out.println("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
