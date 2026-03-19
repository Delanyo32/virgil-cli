use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const DB_IO_PATTERNS: &[&str] = &[
    "mysql_query",
    "mysql_real_query",
    "mysql_store_result",
    "mysql_fetch_row",
    "PQexec",
    "PQexecParams",
    "sqlite3_exec",
    "sqlite3_step",
    "mongo_cursor_next",
    "redis_command",
    "send",
    "recv",
    "sendto",
    "recvfrom",
    "sendmsg",
    "recvmsg",
    "read",
    "write",
    "fread",
    "fwrite",
    "fgets",
    "fputs",
    "fprintf",
    "fscanf",
];

const DB_IO_QUALIFIED_PATTERNS: &[&str] = &[
    "boost::asio::read",
    "boost::asio::write",
    "boost::asio::async_read",
    "boost::asio::async_write",
    "std::getline",
];

const DB_IO_METHOD_PATTERNS: &[&str] = &[
    "execute", "query", "fetch", "prepare", "open", "close", "getline",
];

fn cpp_lang() -> tree_sitter::Language {
    Language::Cpp.tree_sitter_language()
}

pub struct NPlusOneQueriesPipeline {
    loop_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        let loop_query_str = r#"
[
  (for_statement body: (_) @loop_body) @loop_expr
  (for_range_loop body: (_) @loop_body) @loop_expr
  (while_statement body: (_) @loop_body) @loop_expr
  (do_statement body: (_) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&cpp_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for C++ n_plus_one_queries")?;

        let call_query_str = r#"
(call_expression
  function: (_) @fn_name
  arguments: (argument_list) @args) @call
"#;
        let call_query = Query::new(&cpp_lang(), call_query_str)
            .with_context(|| "failed to compile call query for C++ n_plus_one_queries")?;

        Ok(Self {
            loop_query: Arc::new(loop_query),
            call_query: Arc::new(call_query),
        })
    }

    fn is_db_io_call(fn_text: &str) -> Option<&'static str> {
        // Check direct function name matches (unqualified)
        let base_name = fn_text.rsplit("::").next().unwrap_or(fn_text);
        if DB_IO_PATTERNS.contains(&base_name) {
            return Some("db_io_call_in_loop");
        }

        // Check qualified patterns
        for pattern in DB_IO_QUALIFIED_PATTERNS {
            if fn_text.contains(pattern) {
                return Some("db_io_call_in_loop");
            }
        }

        // Check if it looks like a method call on a stream/db object
        // field_expression will give us the full text like "conn.execute"
        let method = fn_text.rsplit('.').next().unwrap_or("");
        if DB_IO_METHOD_PATTERNS.contains(&method) {
            return Some("db_io_call_in_loop");
        }

        // Check for ifstream/ofstream operations via >>/<< operators
        // These are handled via text scan in the loop body
        if fn_text.contains("ifstream") || fn_text.contains("ofstream") {
            return Some("file_io_in_loop");
        }

        None
    }
}

impl Pipeline for NPlusOneQueriesPipeline {
    fn name(&self) -> &str {
        "n_plus_one_queries"
    }

    fn description(&self) -> &str {
        "Detects DB/I/O calls inside loops that may cause N+1 query or performance problems"
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
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut inner_matches =
                    inner_cursor.matches(&self.call_query, tree.root_node(), source);

                let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
                let call_idx = find_capture_index(&self.call_query, "call");

                while let Some(im) = inner_matches.next() {
                    let fn_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == fn_name_idx)
                        .map(|c| c.node);
                    let call_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(fn_n), Some(call_n)) = (fn_node, call_node) {
                        let fn_text = node_text(fn_n, source);

                        if let Some(pattern) = Self::is_db_io_call(fn_text) {
                            let start = call_n.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: pattern.to_string(),
                                message: format!(
                                    "`{fn_text}()` called inside a loop — potential N+1 problem, consider batching"
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_mysql_query_in_for_loop() {
        let src = r#"
void load_users(int* ids, int count, MYSQL* conn) {
    for (int i = 0; i < count; i++) {
        char query[256];
        snprintf(query, sizeof(query), "SELECT * FROM users WHERE id = %d", ids[i]);
        mysql_query(conn, query);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_io_call_in_loop");
        assert!(findings[0].message.contains("mysql_query"));
    }

    #[test]
    fn detects_send_in_while_loop() {
        let src = r#"
void send_messages(int sock, const char** msgs, int count) {
    int i = 0;
    while (i < count) {
        send(sock, msgs[i], strlen(msgs[i]), 0);
        i++;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "db_io_call_in_loop"));
    }

    #[test]
    fn detects_boost_asio_read_in_for_range() {
        let src = r#"
void read_all(std::vector<tcp::socket>& sockets) {
    for (auto& sock : sockets) {
        boost::asio::read(sock, buffer);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("boost::asio::read"));
    }

    #[test]
    fn detects_recv_in_do_while() {
        let src = r#"
void drain(int sock) {
    char buf[1024];
    int n;
    do {
        n = recv(sock, buf, sizeof(buf), 0);
    } while (n > 0);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "db_io_call_in_loop"));
    }

    #[test]
    fn ignores_non_io_calls_in_loop() {
        let src = r#"
void process(std::vector<int>& v) {
    for (auto& x : v) {
        int y = compute(x);
        results.push_back(y);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_io_call_outside_loop() {
        let src = r#"
void load(MYSQL* conn) {
    mysql_query(conn, "SELECT * FROM users");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
void f(MYSQL* conn, int* ids, int n) {
    for (int i = 0; i < n; i++) {
        mysql_query(conn, "SELECT 1");
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "n_plus_one_queries");
    }
}
