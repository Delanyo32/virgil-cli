use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

const BLOCKING_SCOPED_PREFIXES: &[&str] = &[
    "std::fs::read",
    "std::fs::write",
    "std::fs::remove_file",
    "std::fs::create_dir",
    "std::fs::read_to_string",
    "std::fs::copy",
    "std::fs::rename",
    "std::fs::metadata",
    "std::io::stdin",
    "std::net::",
    "std::thread::sleep",
    "thread::sleep",
];

const BLOCKING_METHODS: &[&str] = &["read_to_string", "write_all"];

fn rust_lang() -> tree_sitter::Language {
    Language::Rust.tree_sitter_language()
}

pub struct SyncBlockingInAsyncPipeline {
    fn_query: Arc<Query>,
    scoped_call_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        let fn_query_str = r#"
(function_item
  name: (identifier) @fn_name
  body: (block) @fn_body) @fn_def
"#;
        let fn_query = Query::new(&rust_lang(), fn_query_str)
            .with_context(|| "failed to compile function item query for sync_blocking_in_async")?;

        let scoped_call_query_str = r#"
(call_expression
  function: (scoped_identifier) @scoped_fn) @call
"#;
        let scoped_call_query = Query::new(&rust_lang(), scoped_call_query_str)
            .with_context(|| "failed to compile scoped call query for sync_blocking_in_async")?;

        let method_call_query_str = r#"
(call_expression
  function: (field_expression
    field: (field_identifier) @method_name)) @call
"#;
        let method_call_query = Query::new(&rust_lang(), method_call_query_str)
            .with_context(|| "failed to compile method call query for sync_blocking_in_async")?;

        Ok(Self {
            fn_query: Arc::new(fn_query),
            scoped_call_query: Arc::new(scoped_call_query),
            method_call_query: Arc::new(method_call_query),
        })
    }

    fn find_async_body_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<Range<usize>> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);
        let mut ranges = Vec::new();

        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");
        let body_idx = find_capture_index(&self.fn_query, "fn_body");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_def_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let (Some(fn_cap), Some(body_cap)) = (fn_node, body_node) {
                let fn_text = node_text(fn_cap, source);
                if fn_text.trim_start().starts_with("async") {
                    ranges.push(body_cap.start_byte()..body_cap.end_byte());
                }
            }
        }

        ranges
    }

    fn is_in_async_body(ranges: &[Range<usize>], byte_offset: usize) -> bool {
        ranges.iter().any(|r| r.contains(&byte_offset))
    }
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects blocking I/O and thread::sleep calls inside async functions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let async_ranges = self.find_async_body_ranges(tree, source);
        if async_ranges.is_empty() {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check scoped calls (std::fs::read, std::thread::sleep, etc.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.scoped_call_query, tree.root_node(), source);

            let fn_idx = find_capture_index(&self.scoped_call_query, "scoped_fn");
            let call_idx = find_capture_index(&self.scoped_call_query, "call");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_cap), Some(call_cap)) = (fn_node, call_node) {
                    let fn_text = node_text(fn_cap, source);
                    let is_blocking = BLOCKING_SCOPED_PREFIXES
                        .iter()
                        .any(|prefix| fn_text.starts_with(prefix));

                    if is_blocking && Self::is_in_async_body(&async_ranges, call_cap.start_byte()) {
                        let start = call_cap.start_position();
                        let pattern = if fn_text.contains("thread::sleep") {
                            "thread_sleep_in_async"
                        } else {
                            "blocking_io_in_async"
                        };
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: pattern.to_string(),
                            message: format!(
                                "blocking call `{fn_text}` inside async function — use async equivalent (e.g. `tokio::fs`, `tokio::time::sleep`) to avoid blocking the runtime"
                            ),
                            snippet: extract_snippet(source, call_cap, 1),
                        });
                    }
                }
            }
        }

        // Check blocking method calls (.read_to_string, .write_all)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);

            let name_idx = find_capture_index(&self.method_call_query, "method_name");
            let call_idx = find_capture_index(&self.method_call_query, "call");

            while let Some(m) = matches.next() {
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == name_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(name_cap), Some(call_cap)) = (name_node, call_node) {
                    let method = node_text(name_cap, source);
                    if BLOCKING_METHODS.contains(&method)
                        && Self::is_in_async_body(&async_ranges, call_cap.start_byte())
                    {
                        let start = call_cap.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "blocking_io_in_async".to_string(),
                            message: format!(
                                "blocking method `.{method}()` inside async function — use async I/O instead"
                            ),
                            snippet: extract_snippet(source, call_cap, 1),
                        });
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
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_std_fs_read_in_async() {
        let src = r#"
async fn load_file() {
    let data = std::fs::read("config.toml").unwrap();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_io_in_async");
        assert!(findings[0].message.contains("std::fs::read"));
    }

    #[test]
    fn detects_thread_sleep_in_async() {
        let src = r#"
async fn wait_a_bit() {
    std::thread::sleep(std::time::Duration::from_secs(1));
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "thread_sleep_in_async");
        assert!(findings[0].message.contains("std::thread::sleep"));
    }

    #[test]
    fn detects_write_all_in_async() {
        let src = r#"
async fn save(data: &[u8]) {
    let mut file = std::fs::File::create("out.txt").unwrap();
    file.write_all(data).unwrap();
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "blocking_io_in_async" && f.message.contains("write_all"))
        );
    }

    #[test]
    fn ignores_blocking_in_sync_function() {
        let src = r#"
fn load_file() {
    let data = std::fs::read("config.toml").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_async_io_in_async() {
        let src = r#"
async fn load_file() {
    let data = tokio::fs::read("config.toml").await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
