use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{extract_receiver_text, receiver_matches_any};
use crate::language::Language;
use super::primitives::{extract_snippet, find_capture_index, node_text};

// High-confidence DB methods (always flag)
const DEFINITE_DB_METHODS: &[&str] = &[
    "fetch_one",
    "fetch_all",
    "fetch_optional",
    "query_as",
];

// Ambiguous methods — require receiver heuristic
const MAYBE_DB_METHODS: &[&str] = &["execute", "load", "first", "find", "query"];
const MAYBE_HTTP_METHODS: &[&str] = &["send", "get", "post", "put", "delete"];

// Receiver patterns that indicate DB context
const DB_RECEIVERS: &[&str] = &["conn", "pool", "db", "client", "sqlx", "diesel", "sea_orm", "query", "stmt"];
// Receiver patterns that indicate HTTP context
const HTTP_RECEIVERS: &[&str] = &["client", "reqwest", "http", "hyper"];
// Receiver patterns that are NOT DB/HTTP (skip these)
const NON_DB_RECEIVERS: &[&str] = &["tx", "sender", "mpsc", "map", "iter", "vec", "url", "params", "cache", "arr", "list", "set", "hash", "btree"];

fn rust_lang() -> tree_sitter::Language {
    Language::Rust.tree_sitter_language()
}

pub struct NPlusOneQueriesPipeline {
    loop_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        let loop_query_str = r#"
[
  (for_expression body: (block) @loop_body) @loop_expr
  (while_expression body: (block) @loop_body) @loop_expr
  (loop_expression body: (block) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&rust_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for Rust")?;

        let method_query_str = r#"
(call_expression
  function: (field_expression
    field: (field_identifier) @method_name)) @call
"#;
        let method_query = Query::new(&rust_lang(), method_query_str)
            .with_context(|| "failed to compile method call query for Rust n+1")?;

        Ok(Self {
            loop_query: Arc::new(loop_query),
            method_call_query: Arc::new(method_query),
        })
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
                // Search for method calls inside the loop body
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut inner_matches =
                    inner_cursor.matches(&self.method_call_query, tree.root_node(), source);

                let name_idx = find_capture_index(&self.method_call_query, "method_name");
                let call_idx = find_capture_index(&self.method_call_query, "call");

                while let Some(im) = inner_matches.next() {
                    let name_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == name_idx)
                        .map(|c| c.node);
                    let call_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                        let method_name = node_text(name_n, source);
                        let receiver = extract_receiver_text(call_n, source);

                        let (is_match, pattern) = if DEFINITE_DB_METHODS.contains(&method_name) {
                            (true, "db_query_in_loop")
                        } else if MAYBE_DB_METHODS.contains(&method_name) {
                            // For ambiguous DB methods, check receiver context
                            if receiver_matches_any(receiver, NON_DB_RECEIVERS) {
                                (false, "")
                            } else if receiver_matches_any(receiver, DB_RECEIVERS) {
                                (true, "db_query_in_loop")
                            } else {
                                // Unknown receiver — still flag but could be false positive
                                (true, "db_query_in_loop")
                            }
                        } else if MAYBE_HTTP_METHODS.contains(&method_name) {
                            // For ambiguous HTTP methods, check receiver context
                            if receiver_matches_any(receiver, NON_DB_RECEIVERS) {
                                (false, "")
                            } else if receiver_matches_any(receiver, HTTP_RECEIVERS) {
                                (true, "http_call_in_loop")
                            } else {
                                // Unknown receiver — still flag
                                (true, "http_call_in_loop")
                            }
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
                                    "`.{method_name}()` called inside a loop — potential N+1 query problem, consider batching"
                                ),
                                snippet: extract_snippet(source, loop_n, 5),
                            });
                        }
                    }
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_db_query_in_for_loop() {
        let src = r#"
fn load_users(ids: &[i32], pool: &Pool) {
    for id in ids {
        let user = sqlx::query_as!(User, "SELECT * FROM users WHERE id = $1", id)
            .fetch_one(pool)
            .await;
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
        assert!(findings[0].message.contains("fetch_one"));
    }

    #[test]
    fn detects_http_call_in_while_loop() {
        let src = r#"
fn poll_services(urls: &[String], client: &Client) {
    let mut i = 0;
    while i < urls.len() {
        let resp = client.get(&urls[i]).send().await;
        i += 1;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "http_call_in_loop"));
    }

    #[test]
    fn detects_query_in_loop_expression() {
        let src = r#"
fn retry_query(pool: &Pool) {
    loop {
        let result = conn.execute("INSERT INTO logs VALUES ($1)", &[&msg]);
        if result.is_ok() { break; }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
    }

    #[test]
    fn ignores_non_db_methods_in_loop() {
        let src = r#"
fn process(items: &[Item]) {
    for item in items {
        let name = item.to_string();
        println!("{}", name);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_db_call_outside_loop() {
        let src = r#"
fn load_user(pool: &Pool, id: i32) {
    let user = conn.fetch_one(pool).await;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
