use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Function-call names that indicate a DB query.
const DB_FUNCTIONS: &[&str] = &[
    "mysqli_query",
    "query",
    "prepare",
    "execute",
];

/// Method names on objects that indicate a DB/ORM query.
const DB_METHODS: &[&str] = &[
    "find",
    "get",
    "where",
    "first",
    "all",
    "query",
    "prepare",
    "execute",
];

/// Function-call names that indicate an HTTP request.
const HTTP_FUNCTIONS: &[&str] = &["file_get_contents", "curl_exec"];

/// Method names on objects that indicate an HTTP request.
const HTTP_METHODS: &[&str] = &["get", "request"];

fn php_lang() -> tree_sitter::Language {
    Language::Php.tree_sitter_language()
}

pub struct NPlusOneQueriesPipeline {
    loop_query: Arc<Query>,
    fn_call_query: Arc<Query>,
    member_call_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        let loop_query_str = r#"
[
  (for_statement body: (compound_statement) @loop_body) @loop_expr
  (while_statement body: (compound_statement) @loop_body) @loop_expr
  (foreach_statement body: (compound_statement) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&php_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for PHP n+1")?;

        let fn_call_query_str = r#"
(function_call_expression
  function: (name) @fn_name
  arguments: (arguments) @args) @call
"#;
        let fn_call_query = Query::new(&php_lang(), fn_call_query_str)
            .with_context(|| "failed to compile function_call query for PHP n+1")?;

        let member_call_query_str = r#"
(member_call_expression
  object: (_) @object
  name: (name) @method_name
  arguments: (arguments) @args) @call
"#;
        let member_call_query = Query::new(&php_lang(), member_call_query_str)
            .with_context(|| "failed to compile member_call query for PHP n+1")?;

        Ok(Self {
            loop_query: Arc::new(loop_query),
            fn_call_query: Arc::new(fn_call_query),
            member_call_query: Arc::new(member_call_query),
        })
    }

    fn check_function_calls_in_loop(
        &self,
        tree: &Tree,
        source: &[u8],
        body: tree_sitter::Node,
        loop_node: tree_sitter::Node,
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(body.byte_range());
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

            if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                let fn_name = node_text(name_n, source);

                let (is_match, pattern) = if DB_FUNCTIONS.contains(&fn_name) {
                    (true, "db_query_in_loop")
                } else if HTTP_FUNCTIONS.contains(&fn_name) {
                    (true, "http_call_in_loop")
                } else {
                    (false, "")
                };

                if is_match {
                    let start = call_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: format!(
                            "`{fn_name}()` called inside a loop — potential N+1 query problem, consider batching"
                        ),
                        snippet: extract_snippet(source, loop_node, 5),
                    });
                }
            }
        }
    }

    fn check_method_calls_in_loop(
        &self,
        tree: &Tree,
        source: &[u8],
        body: tree_sitter::Node,
        loop_node: tree_sitter::Node,
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(body.byte_range());
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

            if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                let method_name = node_text(name_n, source);

                let (is_match, pattern) = if DB_METHODS.contains(&method_name) {
                    (true, "db_query_in_loop")
                } else if HTTP_METHODS.contains(&method_name) {
                    (true, "http_call_in_loop")
                } else {
                    (false, "")
                };

                if is_match {
                    let start = call_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: format!(
                            "`->{method_name}()` called inside a loop — potential N+1 query problem, consider batching"
                        ),
                        snippet: extract_snippet(source, loop_node, 5),
                    });
                }
            }
        }
    }
}

impl Pipeline for NPlusOneQueriesPipeline {
    fn name(&self) -> &str {
        "n_plus_one_queries"
    }

    fn description(&self) -> &str {
        "Detects DB/ORM/HTTP calls inside loops that may cause N+1 query problems"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.loop_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.loop_query, "loop_body");
        let loop_idx = find_capture_index(&self.loop_query, "loop_expr");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let loop_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == loop_idx)
                .map(|c| c.node);

            if let (Some(body), Some(loop_n)) = (body_node, loop_node) {
                self.check_function_calls_in_loop(
                    tree, source, body, loop_n, file_path, &mut findings,
                );
                self.check_method_calls_in_loop(
                    tree, source, body, loop_n, file_path, &mut findings,
                );
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_mysqli_query_in_foreach() {
        let src = "<?php\nforeach ($ids as $id) {\n    mysqli_query($conn, \"SELECT * FROM users WHERE id = $id\");\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
        assert!(findings[0].message.contains("mysqli_query"));
    }

    #[test]
    fn detects_orm_find_in_for_loop() {
        let src = "<?php\nfor ($i = 0; $i < count($ids); $i++) {\n    $user = $repo->find($ids[$i]);\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
        assert!(findings[0].message.contains("find"));
    }

    #[test]
    fn detects_http_call_in_while_loop() {
        let src = "<?php\nwhile ($url = array_pop($urls)) {\n    $data = file_get_contents($url);\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "http_call_in_loop");
        assert!(findings[0].message.contains("file_get_contents"));
    }

    #[test]
    fn detects_method_query_in_foreach() {
        let src = "<?php\nforeach ($items as $item) {\n    $result = $db->query(\"SELECT * FROM items WHERE id = $item\");\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
    }

    #[test]
    fn ignores_call_outside_loop() {
        let src = "<?php\n$result = $db->query('SELECT * FROM users');\n$data = file_get_contents('https://example.com');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_db_method_in_loop() {
        let src = "<?php\nforeach ($items as $item) {\n    echo $item->getName();\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_curl_exec_in_foreach() {
        let src = "<?php\nforeach ($urls as $url) {\n    $ch = curl_init($url);\n    $result = curl_exec($ch);\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "http_call_in_loop");
        assert!(findings[0].message.contains("curl_exec"));
    }
}
