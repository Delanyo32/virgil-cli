use std::ops::Range;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

const BLOCKING_SLEEP_PARK_PREFIXES: &[&str] = &["std::thread::sleep", "thread::sleep", "std::thread::park", "thread::park"];

const BLOCKING_SCOPED_PREFIXES: &[&str] =
    &["std::fs::", "fs::", "std::net::", "net::"];

const BLOCKING_METHODS: &[&str] = &["join"];

pub struct AsyncBlockingPipeline {
    fn_query: Arc<Query>,
    async_block_query: Arc<Query>,
    scoped_call_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl AsyncBlockingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: primitives::compile_function_item_query()?,
            async_block_query: primitives::compile_async_block_query()?,
            scoped_call_query: primitives::compile_scoped_call_query()?,
            method_call_query: primitives::compile_method_call_query()?,
        })
    }

    /// Returns byte ranges covering all async fn bodies, excluding test functions.
    fn find_async_fn_body_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<Range<usize>> {
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
                let fn_text = fn_cap.node.utf8_text(source).unwrap_or("");
                if !fn_text.trim_start().starts_with("async") {
                    continue;
                }
                // Skip #[tokio::test] and #[test] annotated functions
                if Self::fn_has_test_attribute(fn_cap.node, source) {
                    continue;
                }
                let start = body_cap.node.start_byte();
                let end = body_cap.node.end_byte();
                ranges.push(start..end);
            }
        }

        ranges
    }

    /// Returns byte ranges covering all `async { }` blocks.
    fn find_async_block_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<Range<usize>> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.async_block_query, tree.root_node(), source);
        let mut ranges = Vec::new();

        let block_idx = self
            .async_block_query
            .capture_names()
            .iter()
            .position(|n| *n == "async_block")
            .unwrap();

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.iter().find(|c| c.index as usize == block_idx) {
                ranges.push(cap.node.start_byte()..cap.node.end_byte());
            }
        }

        ranges
    }

    /// Walk prev_named_siblings of a function_item node to find an attribute_item
    /// whose text contains "tokio::test" or whose text is "#[test]".
    fn fn_has_test_attribute(fn_node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut sibling = fn_node.prev_named_sibling();
        while let Some(sib) = sibling {
            if sib.kind() == "attribute_item" {
                let text = sib.utf8_text(source).unwrap_or("");
                if text.contains("tokio::test") || text.contains("#[test]") {
                    return true;
                }
            } else {
                // Stop walking if we hit something other than attribute_item
                break;
            }
            sibling = sib.prev_named_sibling();
        }
        false
    }

    fn is_in_async_body(ranges: &[Range<usize>], byte_offset: usize) -> bool {
        ranges.iter().any(|r| r.contains(&byte_offset))
    }

    fn severity_for_scoped_call(fn_text: &str) -> &'static str {
        if BLOCKING_SLEEP_PARK_PREFIXES.iter().any(|p| fn_text.starts_with(p)) {
            "error"
        } else {
            "warning"
        }
    }
}

impl GraphPipeline for AsyncBlockingPipeline {
    fn name(&self) -> &str {
        "async_blocking"
    }

    fn description(&self) -> &str {
        "Detects blocking calls (std::fs, thread::sleep, thread::park, .join()) inside async functions or async blocks"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        if is_test_file(ctx.file_path) {
            return Vec::new();
        }

        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        // Collect async regions: fn bodies + async blocks
        let mut async_ranges = self.find_async_fn_body_ranges(tree, source);
        async_ranges.extend(self.find_async_block_ranges(tree, source));

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

                    let is_sleep_park = BLOCKING_SLEEP_PARK_PREFIXES
                        .iter()
                        .any(|prefix| fn_text.starts_with(prefix));
                    let is_other_blocking = BLOCKING_SCOPED_PREFIXES
                        .iter()
                        .any(|prefix| fn_text.starts_with(prefix));

                    if (is_sleep_park || is_other_blocking)
                        && Self::is_in_async_body(&async_ranges, call_cap.node.start_byte())
                    {
                        let severity = Self::severity_for_scoped_call(fn_text);
                        let start = call_cap.node.start_position();
                        let snippet = call_cap.node.utf8_text(source).unwrap_or("").to_string();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: severity.to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "blocking_in_async".to_string(),
                            message: format!(
                                "blocking call `{fn_text}` inside async context — use async equivalent (e.g. `tokio::fs`) to avoid blocking the runtime"
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
                        // Fix: .join(",") on strings/iterators takes arguments;
                        // thread JoinHandle::join() takes NO arguments. Skip if has args.
                        if method == "join"
                            && let Some(args) = call_cap.node.child_by_field_name("arguments")
                            && args.named_child_count() > 0
                        {
                            continue;
                        }
                        // Skip calls inside spawn_blocking/block_in_place closures
                        if crate::audit::pipelines::helpers::is_inside_spawn_blocking(
                            call_cap.node,
                            source,
                        ) {
                            continue;
                        }
                        let start = call_cap.node.start_position();
                        let snippet = call_cap.node.utf8_text(source).unwrap_or("").to_string();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "blocking_in_async".to_string(),
                            message: format!(
                                "blocking call `.{method}()` inside async context — this may block the async runtime"
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
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "src/lib.rs",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn skips_string_join_with_args_in_async() {
        let src = r#"
async fn run() {
    let result = parts.join(",");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_blocking_in_async_block() {
        let src = r#"
fn example() {
    let fut = async {
        let _ = std::fs::read("file.txt");
    };
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty(), "blocking call inside async block should be detected");
    }

    #[test]
    fn tokio_test_fn_not_flagged() {
        let src = r#"
#[tokio::test]
async fn test_read() {
    let _ = std::fs::read("file.txt");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "#[tokio::test] fns should be excluded");
    }

    #[test]
    fn test_attr_fn_not_flagged() {
        let src = r#"
#[test]
async fn test_read() {
    let _ = std::fs::read("file.txt");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "#[test] fns should be excluded");
    }

    #[test]
    fn thread_sleep_in_async_is_error() {
        let src = r#"
async fn handler() {
    std::thread::sleep(std::time::Duration::from_secs(1));
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn fs_read_in_async_is_warning() {
        let src = r#"
async fn handler() {
    let _ = std::fs::read("file.txt");
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn join_in_async_is_info() {
        let src = r#"
async fn run() {
    handle.join();
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn test_file_skipped() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let src = r#"
async fn load() {
    let data = std::fs::read("file.txt");
}
"#;
        let tree = parser.parse(src, None).unwrap();
        let pipeline = AsyncBlockingPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: src.as_bytes(),
            file_path: "src/foo_test.rs",
            id_counts: &id_counts,
            graph: &graph,
        };
        let findings = pipeline.check(&ctx);
        assert!(findings.is_empty(), "test files should be skipped");
    }
}
