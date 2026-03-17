use std::ops::Range;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use super::primitives;

const BLOCKING_SCOPED_PREFIXES: &[&str] = &[
    "std::fs::",
    "fs::",
    "std::thread::sleep",
    "thread::sleep",
];

const BLOCKING_METHODS: &[&str] = &["join"];

pub struct AsyncBlockingPipeline {
    fn_query: Arc<Query>,
    scoped_call_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl AsyncBlockingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: primitives::compile_function_item_query()?,
            scoped_call_query: primitives::compile_scoped_call_query()?,
            method_call_query: primitives::compile_method_call_query()?,
        })
    }

    fn find_async_body_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<Range<usize>> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);
        let mut ranges = Vec::new();

        let fn_def_idx = self
            .fn_query
            .capture_names()
            .iter()
            .position(|n| *n == "fn_def")
            .unwrap();
        let body_idx = self
            .fn_query
            .capture_names()
            .iter()
            .position(|n| *n == "fn_body")
            .unwrap();

        while let Some(m) = matches.next() {
            let fn_node = m.captures.iter().find(|c| c.index as usize == fn_def_idx);
            let body_node = m.captures.iter().find(|c| c.index as usize == body_idx);

            if let (Some(fn_cap), Some(body_cap)) = (fn_node, body_node) {
                // Check if the function_item has an `async` child
                let fn_text = fn_cap.node.utf8_text(source).unwrap_or("");
                // An async fn starts with "async fn"
                if fn_text.trim_start().starts_with("async") {
                    let start = body_cap.node.start_byte();
                    let end = body_cap.node.end_byte();
                    ranges.push(start..end);
                }
            }
        }

        ranges
    }

    fn is_in_async_body(ranges: &[Range<usize>], byte_offset: usize) -> bool {
        ranges.iter().any(|r| r.contains(&byte_offset))
    }
}

impl Pipeline for AsyncBlockingPipeline {
    fn name(&self) -> &str {
        "async_blocking"
    }

    fn description(&self) -> &str {
        "Detects blocking calls (std::fs, thread::sleep, .join()) inside async functions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let async_ranges = self.find_async_body_ranges(tree, source);
        if async_ranges.is_empty() {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check scoped calls (std::fs::read, thread::sleep, etc.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.scoped_call_query, tree.root_node(), source);

            let fn_idx = self
                .scoped_call_query
                .capture_names()
                .iter()
                .position(|n| *n == "scoped_fn")
                .unwrap();
            let call_idx = self
                .scoped_call_query
                .capture_names()
                .iter()
                .position(|n| *n == "call")
                .unwrap();

            while let Some(m) = matches.next() {
                let fn_node = m.captures.iter().find(|c| c.index as usize == fn_idx);
                let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

                if let (Some(fn_cap), Some(call_cap)) = (fn_node, call_node) {
                    let fn_text = fn_cap.node.utf8_text(source).unwrap_or("");
                    let is_blocking = BLOCKING_SCOPED_PREFIXES
                        .iter()
                        .any(|prefix| fn_text.starts_with(prefix));

                    if is_blocking
                        && Self::is_in_async_body(&async_ranges, call_cap.node.start_byte())
                    {
                        let start = call_cap.node.start_position();
                        let snippet =
                            call_cap.node.utf8_text(source).unwrap_or("").to_string();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "blocking_in_async".to_string(),
                            message: format!(
                                "blocking call `{fn_text}` inside async function — use async equivalent (e.g. `tokio::fs`) to avoid blocking the runtime"
                            ),
                            snippet,
                        });
                    }
                }
            }
        }

        // Check method calls (.join(), etc.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);

            let name_idx = self
                .method_call_query
                .capture_names()
                .iter()
                .position(|n| *n == "method_name")
                .unwrap();
            let call_idx = self
                .method_call_query
                .capture_names()
                .iter()
                .position(|n| *n == "call")
                .unwrap();

            while let Some(m) = matches.next() {
                let name_node = m.captures.iter().find(|c| c.index as usize == name_idx);
                let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

                if let (Some(name_cap), Some(call_cap)) = (name_node, call_node) {
                    let method = name_cap.node.utf8_text(source).unwrap_or("");
                    if BLOCKING_METHODS.contains(&method)
                        && Self::is_in_async_body(&async_ranges, call_cap.node.start_byte())
                    {
                        let start = call_cap.node.start_position();
                        let snippet =
                            call_cap.node.utf8_text(source).unwrap_or("").to_string();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "blocking_in_async".to_string(),
                            message: format!(
                                "blocking call `.{method}()` inside async function — this may block the async runtime"
                            ),
                            snippet,
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
        let pipeline = AsyncBlockingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_std_fs_in_async() {
        let src = r#"
async fn load() {
    let data = std::fs::read("file.txt");
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_in_async");
        assert!(findings[0].message.contains("std::fs::read"));
    }

    #[test]
    fn skips_std_fs_in_sync() {
        let src = r#"
fn load() {
    let data = std::fs::read("file.txt");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_tokio_fs_in_async() {
        let src = r#"
async fn load() {
    let data = tokio::fs::read("file.txt").await;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_thread_sleep_in_async() {
        let src = r#"
async fn wait() {
    thread::sleep(Duration::from_secs(1));
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_in_async");
    }

    #[test]
    fn detects_join_in_async() {
        let src = r#"
async fn run() {
    handle.join();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_in_async");
    }
}
