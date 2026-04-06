use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::{is_nolint_suppressed, is_test_file};

use super::primitives::{
    compile_echo_statement_query, compile_function_call_query, compile_member_call_query,
    compile_text_node_query, extract_snippet, find_capture_index, node_text,
};

const DB_FUNCTIONS: &[&str] = &["mysql_query", "mysqli_query", "pg_query", "sqlite_query"];
const DB_METHODS: &[&str] = &["query", "exec", "execute", "prepare", "fetch", "fetchAll"];

/// Object variable names that indicate a database connection/statement.
const DB_OBJECT_PATTERNS: &[&str] = &[
    "$db",
    "$pdo",
    "$conn",
    "$connection",
    "$dbh",
    "$dbc",
    "$mysql",
    "$mysqli",
    "$stmt",
    "$statement",
    "$wpdb",
    "$database",
];

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

impl NodePipeline for LogicInViewsPipeline {
    fn name(&self) -> &str {
        "logic_in_views"
    }

    fn description(&self) -> &str {
        "Detects database calls mixed with HTML output, indicating MVC violations"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return Vec::new();
        }

        // Check if this file has actual HTML output (not just echo of plain text)
        if !has_html_output(tree, source, &self.text_query, &self.echo_query) {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check function calls for DB functions
        self.check_db_function_calls(tree, source, file_path, &mut findings);
        // Check method calls for DB methods (with object name filtering)
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
                    if is_nolint_suppressed(source, call_node, self.name()) {
                        continue;
                    }
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
                if !DB_METHODS.contains(&method_name) {
                    continue;
                }

                // Extract the object variable name to reduce false positives
                // member_call_expression -> object field is the receiver
                let obj_text = call_node
                    .child_by_field_name("object")
                    .map(|obj| node_text(obj, source));

                // Only flag if the object looks like a DB connection
                let is_db_object = match obj_text {
                    Some(text) => DB_OBJECT_PATTERNS
                        .iter()
                        .any(|pattern| text.starts_with(pattern)),
                    None => false,
                };

                if !is_db_object {
                    continue;
                }

                if is_nolint_suppressed(source, call_node, self.name()) {
                    continue;
                }

                let obj_display = obj_text.unwrap_or("$obj");
                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "db_in_view".to_string(),
                    message: format!(
                        "`{obj_display}->{method_name}()` called in a file with HTML output — separate data access from presentation"
                    ),
                    snippet: extract_snippet(source, call_node, 2),
                });
            }
        }
    }
}

/// Check if the file has actual HTML output (text nodes with HTML tags or echo with HTML).
fn has_html_output(tree: &Tree, source: &[u8], text_query: &Query, echo_query: &Query) -> bool {
    // Check for text nodes (inline HTML outside <?php ?> tags)
    // Only count text nodes that actually contain HTML-like content
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(text_query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.first() {
            let text = node_text(cap.node, source);
            // Check if text contains an HTML tag pattern (< followed by a letter)
            if text.bytes().any(|b| b == b'<')
                && has_html_tag(text)
            {
                return true;
            }
        }
    }

    // Check for echo statements that output HTML
    let mut cursor2 = QueryCursor::new();
    let mut matches2 = cursor2.matches(echo_query, tree.root_node(), source);
    while let Some(m) = matches2.next() {
        if let Some(cap) = m.captures.first() {
            let echo_text = node_text(cap.node, source);
            if has_html_tag(echo_text) {
                return true;
            }
        }
    }

    false
}

/// Simple check for HTML tag patterns: `<` followed by a letter.
fn has_html_tag(text: &str) -> bool {
    let bytes = text.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'<' && bytes[i + 1].is_ascii_alphabetic() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "test.php")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = LogicInViewsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_db_call_with_html() {
        let src =
            "<?php\n$rows = mysqli_query($conn, 'SELECT * FROM users');\n?>\n<h1>Users</h1>\n";
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

    // --- New tests ---

    #[test]
    fn skips_non_db_object_method() {
        // $logger->query() should NOT fire -- logger is not a DB object
        let src = "<?php\n$logger->query('SELECT 1');\necho '<h1>Result</h1>';\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "non-DB object method should not be flagged"
        );
    }

    #[test]
    fn detects_pdo_method() {
        let src = "<?php\n$pdo->prepare('SELECT 1');\necho '<h1>Result</h1>';\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn echo_plain_text_not_html() {
        // echo 'plain text' without HTML tags should NOT trigger HTML detection
        let src = "<?php\n$db->query('SELECT 1');\necho 'done';\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "echo of plain text should not be classified as HTML output"
        );
    }

    #[test]
    fn test_file_suppressed() {
        let src =
            "<?php\n$rows = mysqli_query($conn, 'SELECT 1');\n?>\n<h1>Users</h1>\n";
        let findings = parse_and_check_path(src, "tests/ViewTest.php");
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "<?php\n// NOLINT(logic_in_views)\n$rows = mysqli_query($conn, 'SELECT 1');\n?>\n<h1>Users</h1>\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
