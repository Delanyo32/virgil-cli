use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Blocking function calls that are problematic in long-running PHP processes
/// (workers, daemons, ReactPHP, Swoole, etc.).
const BLOCKING_FUNCTIONS: &[&str] = &[
    "sleep",
    "usleep",
    "time_nanosleep",
    "file_get_contents",
    "fread",
    "fwrite",
    "fgets",
    "file_put_contents",
    "stream_get_contents",
];

fn php_lang() -> tree_sitter::Language {
    Language::Php.tree_sitter_language()
}

pub struct SyncBlockingInAsyncPipeline {
    fn_call_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        let fn_call_query_str = r#"
(function_call_expression
  function: (name) @fn_name
  arguments: (arguments) @args) @call
"#;
        let fn_call_query = Query::new(&php_lang(), fn_call_query_str)
            .with_context(|| "failed to compile function_call query for PHP sync_blocking")?;

        Ok(Self {
            fn_call_query: Arc::new(fn_call_query),
        })
    }

    /// Returns true if `node` is inside a callback-style context:
    /// an anonymous function or arrow function passed as an argument, or inside
    /// a method call on an event loop / promise-like object.
    fn is_inside_callback(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            match parent.kind() {
                "anonymous_function" | "arrow_function" => {
                    return true;
                }
                _ => {}
            }
            current = parent.parent();
        }
        false
    }
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects blocking calls (sleep, file I/O) that are problematic in long-running processes or callback contexts"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
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

            if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                let fn_name = node_text(name_n, source);

                if !BLOCKING_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                // Only flag if inside a callback context (anonymous function, arrow function).
                // This reduces noise for standard synchronous PHP scripts.
                if !Self::is_inside_callback(call_n) {
                    continue;
                }

                let start = call_n.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "blocking_call".to_string(),
                    message: format!(
                        "`{fn_name}()` is a blocking call inside a callback — may block event loop or long-running process"
                    ),
                    snippet: extract_snippet(source, call_n, 2),
                });
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
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_sleep_in_anonymous_function() {
        let src = "<?php\n$loop->addTimer(1.0, function () {\n    sleep(5);\n});\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_call");
        assert!(findings[0].message.contains("sleep"));
    }

    #[test]
    fn detects_file_get_contents_in_callback() {
        let src =
            "<?php\n$promise->then(function ($url) {\n    $data = file_get_contents($url);\n});\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("file_get_contents"));
    }

    #[test]
    fn detects_fread_in_arrow_function() {
        let src = "<?php\n$handler = fn($fp) => fread($fp, 1024);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("fread"));
    }

    #[test]
    fn ignores_sleep_at_top_level() {
        let src = "<?php\nsleep(1);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_blocking_call_in_callback() {
        let src = "<?php\n$loop->addTimer(1.0, function () {\n    echo 'hello';\n});\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_blocking_call_in_regular_function() {
        let src = "<?php\nfunction download($url) {\n    return file_get_contents($url);\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
