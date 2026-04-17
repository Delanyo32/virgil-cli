// PERMANENT RUST EXCEPTION: This pipeline requires FlowsTo/SanitizedBy graph
// predicates for taint propagation analysis. These are not expressible in the
// match_pattern JSON DSL. Do not migrate -- this file stays as Rust intentionally.
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_call_query, compile_member_call_query, extract_snippet, find_capture_index,
    node_text,
};

const SQL_FUNCTIONS: &[&str] = &["mysql_query", "mysqli_query", "pg_query", "sqlite_query"];
const SQL_METHODS: &[&str] = &["query", "exec"];

pub struct SqlInjectionPipeline {
    call_query: Arc<Query>,
    member_query: Arc<Query>,
}

impl SqlInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_function_call_query()?,
            member_query: compile_member_call_query()?,
        })
    }
}

impl Pipeline for SqlInjectionPipeline {
    fn name(&self) -> &str {
        "sql_injection"
    }

    fn description(&self) -> &str {
        "Detects SQL queries built with string concatenation or interpolation"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_function_calls(tree, source, file_path, &mut findings);
        self.check_method_calls(tree, source, file_path, &mut findings);
        findings
    }
}

impl SqlInjectionPipeline {
    fn check_function_calls(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(args_node), Some(call_node)) =
                (name_node, args_node, call_node)
            {
                let fn_name = node_text(name_node, source);
                if !SQL_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                if contains_concatenation(args_node, source) {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_concatenation".to_string(),
                        message: format!(
                            "`{fn_name}()` uses string concatenation/interpolation — use parameterized queries"
                        ),
                        snippet: extract_snippet(source, call_node, 2),
                    });
                }
            }
        }
    }

    fn check_method_calls(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.member_query, tree.root_node(), source);

        let method_name_idx = find_capture_index(&self.member_query, "method_name");
        let args_idx = find_capture_index(&self.member_query, "args");
        let call_idx = find_capture_index(&self.member_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(args_node), Some(call_node)) =
                (name_node, args_node, call_node)
            {
                let method_name = node_text(name_node, source);
                if !SQL_METHODS.contains(&method_name) {
                    continue;
                }

                if contains_concatenation(args_node, source) {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "sql_concatenation".to_string(),
                        message: format!(
                            "`->{}()` uses string concatenation/interpolation — use parameterized queries",
                            method_name
                        ),
                        snippet: extract_snippet(source, call_node, 2),
                    });
                }
            }
        }
    }
}

fn contains_concatenation(node: tree_sitter::Node, source: &[u8]) -> bool {
    // Walk the subtree looking for concatenation or interpolation
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        match current.kind() {
            "binary_expression" => {
                // Check if this is a string concatenation (. operator)
                // The operator is an anonymous child between the operands
                let text = node_text(current, source);
                if text.contains('.') {
                    return true;
                }
            }
            "encapsed_string" => {
                // Double-quoted string with variable interpolation
                return true;
            }
            _ => {}
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SqlInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_concat_in_mysql_query() {
        let src = "<?php\nmysqli_query($conn, 'SELECT * FROM users WHERE id=' . $id);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_concatenation");
    }

    #[test]
    fn detects_interpolation_in_query() {
        let src = "<?php\n$db->query(\"SELECT * FROM users WHERE id=$id\");\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_parameterized_query() {
        let src = "<?php\n$stmt = $db->query('SELECT * FROM users WHERE id = ?');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_non_sql_function() {
        let src = "<?php\nstr_replace('a' . 'b', 'c', $str);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
