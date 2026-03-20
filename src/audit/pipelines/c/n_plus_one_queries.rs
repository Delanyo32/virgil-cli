use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const IO_FUNCTIONS: &[&str] = &[
    "fopen", "fread", "fwrite", "fgets", "fputs", "fgetc", "fputc", "recv", "send", "read",
    "write", "connect", "accept", "select", "recvfrom", "sendto", "recvmsg", "sendmsg", "pread",
    "pwrite",
];

const DB_FUNCTIONS: &[&str] = &[
    "mysql_query",
    "mysql_real_query",
    "mysql_store_result",
    "sqlite3_exec",
    "sqlite3_step",
    "sqlite3_prepare",
    "PQexec",
    "PQexecParams",
    "PQsendQuery",
];

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

pub struct NPlusOneQueriesPipeline {
    loop_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl NPlusOneQueriesPipeline {
    pub fn new() -> Result<Self> {
        let loop_query_str = r#"
[
  (for_statement body: (compound_statement) @loop_body) @loop_expr
  (while_statement body: (compound_statement) @loop_body) @loop_expr
  (do_statement body: (compound_statement) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&c_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for C n_plus_one_queries")?;

        let call_query_str = r#"
(call_expression
  function: (identifier) @fn_name) @call
"#;
        let call_query = Query::new(&c_lang(), call_query_str)
            .with_context(|| "failed to compile call query for C n_plus_one_queries")?;

        Ok(Self {
            loop_query: Arc::new(loop_query),
            call_query: Arc::new(call_query),
        })
    }
}

impl Pipeline for NPlusOneQueriesPipeline {
    fn name(&self) -> &str {
        "n_plus_one_queries"
    }

    fn description(&self) -> &str {
        "Detects I/O and database calls inside loops that may cause N+1 performance problems"
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

                let name_idx = find_capture_index(&self.call_query, "fn_name");
                let call_idx = find_capture_index(&self.call_query, "call");

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
                        let fn_name = node_text(name_n, source);

                        let (is_match, pattern) = if DB_FUNCTIONS.contains(&fn_name) {
                            (true, "db_query_in_loop")
                        } else if IO_FUNCTIONS.contains(&fn_name) {
                            (true, "io_call_in_loop")
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
                                    "`{fn_name}()` called inside a loop — potential N+1 problem, consider batching"
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NPlusOneQueriesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_db_query_in_for_loop() {
        let src = r#"
void process_ids(int *ids, int n) {
    for (int i = 0; i < n; i++) {
        mysql_query(conn, query);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
        assert!(findings[0].message.contains("mysql_query"));
    }

    #[test]
    fn detects_io_call_in_while_loop() {
        let src = r#"
void read_all(int *fds, int n) {
    int i = 0;
    while (i < n) {
        recv(fds[i], buf, sizeof(buf), 0);
        i++;
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "io_call_in_loop");
        assert!(findings[0].message.contains("recv"));
    }

    #[test]
    fn detects_fopen_in_do_while() {
        let src = r#"
void open_files(char **names, int n) {
    int i = 0;
    do {
        fopen(names[i], "r");
        i++;
    } while (i < n);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "io_call_in_loop");
        assert!(findings[0].message.contains("fopen"));
    }

    #[test]
    fn detects_sqlite_in_loop() {
        let src = r#"
void insert_rows(sqlite3 *db, char **rows, int n) {
    for (int i = 0; i < n; i++) {
        sqlite3_exec(db, rows[i], NULL, NULL, NULL);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "db_query_in_loop");
        assert!(findings[0].message.contains("sqlite3_exec"));
    }

    #[test]
    fn ignores_non_io_calls_in_loop() {
        let src = r#"
void process(int *items, int n) {
    for (int i = 0; i < n; i++) {
        printf("%d\n", items[i]);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_io_call_outside_loop() {
        let src = r#"
void single_read(int fd) {
    recv(fd, buf, sizeof(buf), 0);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
