use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::php_primitives::{
    compile_echo_statement_query, compile_function_call_query, compile_member_call_query,
    compile_text_node_query, extract_snippet, find_capture_index, node_text,
};

const DB_FUNCTIONS: &[&str] = &["mysql_query", "mysqli_query", "pg_query", "sqlite_query"];
const DB_METHODS: &[&str] = &["query", "exec", "execute", "prepare", "fetch", "fetchAll"];

pub struct LogicInViewsPipeline {
    fn_call_query: Arc<Query>,
    member_call_query: Arc<Query>,
    echo_query: Arc<Query>,
    text_query: Arc<Query>,
}

impl LogicInViewsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_call_query: compile_function_call_query()?,
            member_call_query: compile_member_call_query()?,
            echo_query: compile_echo_statement_query()?,
            text_query: compile_text_node_query()?,
        })
    }
}

impl Pipeline for LogicInViewsPipeline {
    fn name(&self) -> &str {
        "logic_in_views"
    }

    fn description(&self) -> &str {
        "Detects database calls mixed with HTML output, indicating MVC violations"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        // First check if this file has HTML output (text nodes or echo statements)
        if !has_html_output(tree, source, &self.text_query, &self.echo_query) {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check function calls for DB functions
        self.check_db_function_calls(tree, source, file_path, &mut findings);
        // Check method calls for DB methods
        self.check_db_method_calls(tree, source, file_path, &mut findings);

        findings
    }
}

impl LogicInViewsPipeline {
    fn check_db_function_calls(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_call_query, "fn_name");
        let call_idx = find_capture_index(&self.fn_call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(call_node)) = (name_node, call_node) {
                let fn_name = node_text(name_node, source);
                if DB_FUNCTIONS.contains(&fn_name) {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "db_in_view".to_string(),
                        message: format!(
                            "`{fn_name}()` called in a file with HTML output — separate data access from presentation"
                        ),
                        snippet: extract_snippet(source, call_node, 2),
                    });
                }
            }
        }
    }

    fn check_db_method_calls(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.member_call_query, tree.root_node(), source);

        let method_name_idx = find_capture_index(&self.member_call_query, "method_name");
        let call_idx = find_capture_index(&self.member_call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(call_node)) = (name_node, call_node) {
                let method_name = node_text(name_node, source);
                if DB_METHODS.contains(&method_name) {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "db_in_view".to_string(),
                        message: format!(
                            "`->{method_name}()` called in a file with HTML output — separate data access from presentation"
                        ),
                        snippet: extract_snippet(source, call_node, 2),
                    });
                }
            }
        }
    }
}

fn has_html_output(
    tree: &Tree,
    source: &[u8],
    text_query: &Query,
    echo_query: &Query,
) -> bool {
    // Check for text nodes (inline HTML)
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(text_query, tree.root_node(), source);
    if matches.next().is_some() {
        return true;
    }

    // Check for echo statements
    let mut cursor2 = QueryCursor::new();
    let mut matches2 = cursor2.matches(echo_query, tree.root_node(), source);
    matches2.next().is_some()
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
        let pipeline = LogicInViewsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_db_call_with_html() {
        let src = "<?php\n$rows = mysqli_query($conn, 'SELECT * FROM users');\n?>\n<h1>Users</h1>\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_in_view");
    }

    #[test]
    fn detects_method_call_with_echo() {
        let src = "<?php\n$rows = $db->query('SELECT 1');\necho '<h1>Result</h1>';\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_in_view");
    }

    #[test]
    fn clean_no_html_output() {
        let src = "<?php\n$rows = $db->query('SELECT 1');\nreturn $rows;\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_db_calls() {
        let src = "<?php\necho '<h1>Hello</h1>';\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
