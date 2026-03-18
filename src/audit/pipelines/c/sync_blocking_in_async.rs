use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const BLOCKING_FUNCTIONS: &[&str] = &[
    "sleep", "usleep", "nanosleep",
    "fread", "fwrite", "fgets", "fputs",
    "recv", "send", "recvfrom", "sendto",
    "accept", "connect",
    "read", "write",
    "pread", "pwrite",
    "getaddrinfo",
    "gethostbyname",
    "select", "poll",
    "waitpid", "wait",
    "system",
];

const SLEEP_FUNCTIONS: &[&str] = &["sleep", "usleep", "nanosleep"];

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

pub struct SyncBlockingInAsyncPipeline {
    fn_def_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        let fn_def_query_str = r#"
(function_definition
  declarator: (_) @declarator
  body: (compound_statement) @fn_body) @fn_def
"#;
        let fn_def_query = Query::new(&c_lang(), fn_def_query_str)
            .with_context(|| "failed to compile function_definition query for sync_blocking_in_async")?;

        let call_query_str = r#"
(call_expression
  function: (identifier) @fn_name) @call
"#;
        let call_query = Query::new(&c_lang(), call_query_str)
            .with_context(|| "failed to compile call query for sync_blocking_in_async")?;

        Ok(Self {
            fn_def_query: Arc::new(fn_def_query),
            call_query: Arc::new(call_query),
        })
    }

    /// Check if a function name looks like a callback (common C patterns).
    /// Heuristic: name contains "callback", "handler", "cb", "on_", "_cb",
    /// "_handler", or the function is passed as a pointer (detected via naming).
    fn is_callback_name(name: &str) -> bool {
        let lower = name.to_lowercase();
        lower.contains("callback")
            || lower.contains("handler")
            || lower.ends_with("_cb")
            || lower.starts_with("on_")
            || lower.contains("_handler")
            || lower.contains("_hook")
            || lower.contains("_notify")
            || lower.contains("_listener")
            || lower.contains("event_")
    }

    /// Extract the function name from a declarator node, handling pointer
    /// declarators and function declarators.
    fn extract_fn_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        match node.kind() {
            "identifier" => node.utf8_text(source).ok().map(|s| s.to_string()),
            "function_declarator" => {
                if let Some(declarator) = node.child_by_field_name("declarator") {
                    Self::extract_fn_name(declarator, source)
                } else {
                    None
                }
            }
            "pointer_declarator" => {
                if let Some(declarator) = node.child_by_field_name("declarator") {
                    Self::extract_fn_name(declarator, source)
                } else {
                    None
                }
            }
            "parenthesized_declarator" => {
                // Handle (*fn_ptr)(args) style
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if let Some(name) = Self::extract_fn_name(child, source) {
                        return Some(name);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects blocking I/O and sleep calls in callback-style functions and general blocking patterns"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let declarator_idx = find_capture_index(&self.fn_def_query, "declarator");
        let body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let declarator_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == declarator_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let (Some(declarator), Some(body)) = (declarator_node, body_node) {
                let fn_name = Self::extract_fn_name(declarator, source)
                    .unwrap_or_default();
                let is_callback = Self::is_callback_name(&fn_name);

                // Search for blocking calls inside this function body
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
                        let called_fn = node_text(name_n, source);

                        // Always flag sleep functions, flag other blocking calls
                        // only inside callback-style functions
                        let is_sleep = SLEEP_FUNCTIONS.contains(&called_fn);
                        let is_blocking = BLOCKING_FUNCTIONS.contains(&called_fn);

                        if is_sleep || (is_callback && is_blocking) {
                            let start = call_n.start_position();
                            let message = if is_sleep && is_callback {
                                format!(
                                    "`{called_fn}()` blocks inside callback `{fn_name}` — this will stall the event loop"
                                )
                            } else if is_sleep {
                                format!(
                                    "`{called_fn}()` is a blocking call — consider non-blocking alternatives in server code"
                                )
                            } else {
                                format!(
                                    "`{called_fn}()` is a blocking call inside callback `{fn_name}` — this may stall the event loop"
                                )
                            };

                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "blocking_call".to_string(),
                                message,
                                snippet: extract_snippet(source, call_n, 1),
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
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_sleep_in_any_function() {
        let src = r#"
void process_request(int fd) {
    sleep(5);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_call");
        assert!(findings[0].message.contains("sleep"));
    }

    #[test]
    fn detects_usleep_in_any_function() {
        let src = r#"
void do_work() {
    usleep(1000);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("usleep"));
    }

    #[test]
    fn detects_blocking_recv_in_callback() {
        let src = r#"
void on_connection_cb(int fd) {
    char buf[1024];
    recv(fd, buf, sizeof(buf), 0);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_call");
        assert!(findings[0].message.contains("recv"));
        assert!(findings[0].message.contains("on_connection_cb"));
    }

    #[test]
    fn detects_blocking_in_event_handler() {
        let src = r#"
void event_handler(int fd, void *data) {
    fread(data, 1, 1024, fp);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("fread"));
    }

    #[test]
    fn ignores_blocking_in_regular_function() {
        let src = r#"
void process_file(const char *path) {
    fread(buf, 1, size, fp);
    recv(fd, buf, sizeof(buf), 0);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_blocking_calls() {
        let src = r#"
void on_data_cb(int fd) {
    printf("received data\n");
    memcpy(dest, src, n);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
