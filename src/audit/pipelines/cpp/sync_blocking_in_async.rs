use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Blocking calls that should not appear in coroutine bodies
const BLOCKING_CALL_PATTERNS: &[&str] = &[
    "std::this_thread::sleep_for",
    "std::this_thread::sleep_until",
    "sleep",
    "usleep",
    "nanosleep",
    "system",
];

/// Blocking method calls (member function names)
const BLOCKING_METHOD_PATTERNS: &[&str] = &[
    "get",  // std::future::get() without timeout
    "wait", // std::future::wait()
    "join", // std::thread::join()
];

/// Blocking I/O identifiers that suggest synchronous reads in coroutine context
const BLOCKING_IO_PATTERNS: &[&str] = &[
    "std::cin", "getline", "scanf", "getchar", "fgets", "fread", "fwrite",
];

fn cpp_lang() -> tree_sitter::Language {
    Language::Cpp.tree_sitter_language()
}

pub struct SyncBlockingInAsyncPipeline {
    fn_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        let fn_query_str = r#"
(function_definition
  declarator: (_) @declarator
  body: (compound_statement) @fn_body) @fn_def
"#;
        let fn_query = Query::new(&cpp_lang(), fn_query_str)
            .with_context(|| "failed to compile function query for C++ sync_blocking_in_async")?;

        let call_query_str = r#"
(call_expression
  function: (_) @fn_name
  arguments: (argument_list) @args) @call
"#;
        let call_query = Query::new(&cpp_lang(), call_query_str)
            .with_context(|| "failed to compile call query for C++ sync_blocking_in_async")?;

        Ok(Self {
            fn_query: Arc::new(fn_query),
            call_query: Arc::new(call_query),
        })
    }

    /// Detect coroutine bodies by scanning for co_await, co_return, co_yield keywords
    fn find_coroutine_body_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.fn_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let Some(body) = body_node {
                let body_text = node_text(body, source);
                // A function is a coroutine if its body contains co_await, co_return, or co_yield
                if body_text.contains("co_await")
                    || body_text.contains("co_return")
                    || body_text.contains("co_yield")
                {
                    ranges.push(body.start_byte()..body.end_byte());
                }
            }
        }

        ranges
    }

    fn is_in_coroutine(ranges: &[Range<usize>], byte_offset: usize) -> bool {
        ranges.iter().any(|r| r.contains(&byte_offset))
    }

    /// Check if `fn_text` matches a blocking pattern.
    /// For qualified patterns (containing `::`) we use substring matching.
    /// For short/unqualified patterns we require an exact match against the
    /// full text or the last `::` segment so that e.g. `async_sleep` does not
    /// falsely match the `sleep` pattern.
    fn matches_pattern(fn_text: &str, pattern: &str) -> bool {
        if pattern.contains("::") {
            // Qualified name — substring match is fine
            fn_text.contains(pattern)
        } else {
            // Unqualified name — must be an exact match or the trailing segment
            fn_text == pattern || fn_text.rsplit("::").next() == Some(pattern)
        }
    }

    fn classify_blocking_call(fn_text: &str) -> Option<(&'static str, &'static str)> {
        // Check direct blocking call patterns
        for pattern in BLOCKING_CALL_PATTERNS {
            if Self::matches_pattern(fn_text, pattern) {
                let p = if pattern.contains("sleep") {
                    "sleep_in_coroutine"
                } else {
                    "blocking_call_in_coroutine"
                };
                return Some((p, pattern));
            }
        }

        // Check blocking I/O patterns
        for pattern in BLOCKING_IO_PATTERNS {
            if Self::matches_pattern(fn_text, pattern) {
                return Some(("blocking_io_in_coroutine", pattern));
            }
        }

        // Check blocking method patterns (e.g., future.get())
        let method = fn_text.rsplit('.').next().unwrap_or("");
        let base = fn_text.rsplit("::").next().unwrap_or(fn_text);
        for pattern in BLOCKING_METHOD_PATTERNS {
            if method == *pattern || base == *pattern {
                return Some(("blocking_wait_in_coroutine", pattern));
            }
        }

        None
    }
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects blocking calls inside C++20 coroutine bodies (co_await/co_return/co_yield)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let coroutine_ranges = self.find_coroutine_body_ranges(tree, source);
        if coroutine_ranges.is_empty() {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(fn_n), Some(call_n)) = (fn_node, call_node) {
                if !Self::is_in_coroutine(&coroutine_ranges, call_n.start_byte()) {
                    continue;
                }

                let fn_text = node_text(fn_n, source);

                if let Some((pattern, matched)) = Self::classify_blocking_call(fn_text) {
                    let start = call_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: format!(
                            "blocking call `{matched}` inside coroutine — use async equivalent to avoid blocking the coroutine executor"
                        ),
                        snippet: extract_snippet(source, call_n, 1),
                    });
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
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_sleep_in_coroutine() {
        let src = r#"
Task<void> process() {
    co_await init();
    std::this_thread::sleep_for(std::chrono::seconds(1));
    co_return;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sleep_in_coroutine");
        assert!(findings[0].message.contains("sleep"));
    }

    #[test]
    fn detects_blocking_io_in_coroutine() {
        let src = r#"
Task<std::string> read_input() {
    co_await ready();
    char buf[256];
    fgets(buf, sizeof(buf), stdin);
    co_return std::string(buf);
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "blocking_io_in_coroutine")
        );
    }

    #[test]
    fn detects_future_get_in_coroutine() {
        let src = r#"
Task<int> compute() {
    auto future = std::async(std::launch::async, heavy_work);
    int result = future.get();
    co_return result;
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "blocking_wait_in_coroutine")
        );
    }

    #[test]
    fn ignores_blocking_in_regular_function() {
        let src = r#"
void process() {
    std::this_thread::sleep_for(std::chrono::seconds(1));
    auto result = future.get();
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_async_calls_in_coroutine() {
        let src = r#"
Task<void> process() {
    co_await async_sleep(std::chrono::seconds(1));
    auto result = co_await async_compute();
    co_return;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_scanf_in_coroutine() {
        let src = r#"
Task<int> read_value() {
    int x;
    scanf("%d", &x);
    co_return x;
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "blocking_io_in_coroutine")
        );
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
Task<void> f() {
    co_await init();
    std::this_thread::sleep_for(std::chrono::seconds(1));
    co_return;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "sync_blocking_in_async");
    }
}
