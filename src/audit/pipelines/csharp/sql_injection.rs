use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_invocation_query, compile_object_creation_query, extract_snippet, find_capture_index,
    node_text,
};

pub struct SqlInjectionPipeline {
    creation_query: Arc<Query>,
    invocation_query: Arc<Query>,
}

impl SqlInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            creation_query: compile_object_creation_query()?,
            invocation_query: compile_invocation_query()?,
        })
    }
}

impl Pipeline for SqlInjectionPipeline {
    fn name(&self) -> &str {
        "sql_injection"
    }

    fn description(&self) -> &str {
        "Detects SQL injection risks: SqlCommand with string concatenation, interpolation, or String.Format"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_sql_command(tree, source, file_path, &mut findings);
        self.check_execute_methods(tree, source, file_path, &mut findings);
        findings
    }
}

impl SqlInjectionPipeline {
    fn check_sql_command(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.creation_query, tree.root_node(), source);

        let type_idx = find_capture_index(&self.creation_query, "type_name");
        let args_idx = find_capture_index(&self.creation_query, "args");
        let creation_idx = find_capture_index(&self.creation_query, "creation");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(args_node), Some(creation_node)) =
                (type_node, args_node, creation_node)
            {
                let type_name = node_text(type_node, source);
                if type_name != "SqlCommand"
                    && type_name != "MySqlCommand"
                    && type_name != "NpgsqlCommand"
                {
                    continue;
                }

                let args_text = node_text(args_node, source);

                if args_text.contains("string.Format") || args_text.contains("String.Format") {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_format".to_string(),
                        message: format!(
                            "new {type_name}() uses String.Format — use parameterized queries with @param"
                        ),
                        snippet: extract_snippet(source, creation_node, 2),
                    });
                } else if contains_interpolation(args_node) {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_interpolation".to_string(),
                        message: format!(
                            "new {type_name}() uses string interpolation — use parameterized queries with @param"
                        ),
                        snippet: extract_snippet(source, creation_node, 2),
                    });
                } else if args_text.contains('+') {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_concat".to_string(),
                        message: format!(
                            "new {type_name}() uses string concatenation — use parameterized queries with @param"
                        ),
                        snippet: extract_snippet(source, creation_node, 2),
                    });
                }
            }
        }
    }

    fn check_execute_methods(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
        let args_idx = find_capture_index(&self.invocation_query, "args");
        let inv_idx = find_capture_index(&self.invocation_query, "invocation");

        let sql_methods = ["ExecuteSqlRaw", "FromSqlRaw", "SqlQuery"];

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(inv_node)) = (fn_node, args_node, inv_node)
            {
                let fn_text = node_text(fn_node, source);
                let matches_method = sql_methods.iter().any(|m| fn_text.contains(m));
                if !matches_method {
                    continue;
                }

                let args_text = node_text(args_node, source);
                if args_text.contains('+') || contains_interpolation(args_node) {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_string_concat".to_string(),
                        message: "Raw SQL method with dynamic query — use parameterized queries"
                            .to_string(),
                        snippet: extract_snippet(source, inv_node, 2),
                    });
                }
            }
        }
    }
}

fn contains_interpolation(node: tree_sitter::Node) -> bool {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind() == "interpolated_string_expression" {
            return true;
        }
        for i in 0..current.named_child_count() {
            if let Some(child) = current.named_child(i) {
                stack.push(child);
            }
        }
    }
    false
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
        let pipeline = SqlInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_sql_concat() {
        let src = r#"class Dao {
    void Find(string id) {
        new SqlCommand("SELECT * FROM users WHERE id = " + id, conn);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_string_concat");
    }

    #[test]
    fn detects_sql_interpolation() {
        let src = r#"class Dao {
    void Find(string id) {
        new SqlCommand($"SELECT * FROM users WHERE id = {id}", conn);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_string_interpolation");
    }

    #[test]
    fn ignores_parameterized_query() {
        let src = r#"class Dao {
    void Find(string id) {
        new SqlCommand("SELECT * FROM users WHERE id = @id", conn);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_sql() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
