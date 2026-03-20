use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

/// Specific obj.method blocking calls.
const BLOCKING_ATTR_CALLS: &[(&str, &str)] = &[
    ("time", "sleep"),
    ("os", "read"),
    ("os", "write"),
    ("subprocess", "run"),
    ("subprocess", "call"),
    ("subprocess", "check_output"),
    ("subprocess", "check_call"),
    ("requests", "get"),
    ("requests", "post"),
    ("requests", "put"),
    ("requests", "delete"),
    ("requests", "patch"),
    ("socket", "connect"),
    ("socket", "recv"),
    ("socket", "send"),
];

/// Bare function calls that block.
const BLOCKING_BARE_CALLS: &[&str] = &["open", "input", "sleep"];

pub struct SyncBlockingInAsyncPipeline {
    call_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }

    /// Walk up from `node` to determine if it is inside an `async def` function body.
    /// Returns true if an ancestor is a `function_definition` whose source text starts with `async`.
    ///
    /// In tree-sitter-python, `async def foo(): ...` produces a `function_definition` node
    /// that spans from the `async` keyword through the end of the function body.
    /// We check the source text rather than relying on specific child node structure,
    /// matching the approach used by the TypeScript pipeline.
    fn is_inside_async_function(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_definition" {
                let fn_text = node_text(parent, source);
                if fn_text.trim_start().starts_with("async") {
                    return true;
                }
                // We found the enclosing function and it's not async — stop here.
                return false;
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
        "Detects synchronous blocking calls inside async function definitions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(call_node)) = (fn_node, call_node) {
                if !Self::is_inside_async_function(call_node, source) {
                    continue;
                }

                let mut matched_call: Option<String> = None;

                if fn_node.kind() == "attribute" {
                    let obj = fn_node
                        .child_by_field_name("object")
                        .map(|n| node_text(n, source));
                    let attr = fn_node
                        .child_by_field_name("attribute")
                        .map(|n| node_text(n, source));

                    if let (Some(obj), Some(attr)) = (obj, attr) {
                        for &(expected_obj, expected_method) in BLOCKING_ATTR_CALLS {
                            if obj == expected_obj && attr == expected_method {
                                matched_call = Some(format!("{obj}.{attr}"));
                                break;
                            }
                        }
                    }
                } else if fn_node.kind() == "identifier" {
                    let fn_name = node_text(fn_node, source);
                    if BLOCKING_BARE_CALLS.contains(&fn_name) {
                        matched_call = Some(fn_name.to_string());
                    }
                }

                if let Some(call_name) = matched_call {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "blocking_in_async_def".to_string(),
                        message: format!(
                            "`{call_name}()` is a blocking call inside an async function — use an async equivalent or run in executor"
                        ),
                        snippet: extract_snippet(source, call_node, 1),
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_time_sleep_in_async() {
        let src = "\
async def handler():
    time.sleep(1)
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_in_async_def");
        assert!(findings[0].message.contains("time.sleep"));
    }

    #[test]
    fn detects_requests_get_in_async() {
        let src = "\
async def fetch_data():
    resp = requests.get(url)
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("requests.get"));
    }

    #[test]
    fn detects_open_in_async() {
        let src = "\
async def read_file():
    f = open('data.txt')
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("open"));
    }

    #[test]
    fn ignores_blocking_call_in_sync_function() {
        let src = "\
def handler():
    time.sleep(1)
    requests.get(url)
    open('data.txt')
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_blocking_call_in_async() {
        let src = "\
async def handler():
    await asyncio.sleep(1)
    result = process(data)
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_subprocess_run_in_async() {
        let src = "\
async def deploy():
    subprocess.run(['git', 'push'])
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("subprocess.run"));
    }
}
