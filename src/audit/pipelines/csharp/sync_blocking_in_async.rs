use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, has_modifier, node_text};

fn csharp_lang() -> tree_sitter::Language {
    Language::CSharp.tree_sitter_language()
}

pub struct SyncBlockingInAsyncPipeline {
    method_query: Arc<Query>,
    member_access_query: Arc<Query>,
    invocation_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        let method_query_str = r#"
(method_declaration
  returns: (_) @return_type
  name: (identifier) @method_name
  body: (block) @method_body) @method_decl
"#;
        let method_query = Query::new(&csharp_lang(), method_query_str)
            .with_context(|| "failed to compile method query for sync_blocking_in_async")?;

        let member_access_query_str = r#"
(member_access_expression
  name: (identifier) @member_name) @member_access
"#;
        let member_access_query = Query::new(&csharp_lang(), member_access_query_str)
            .with_context(|| "failed to compile member_access query for sync_blocking_in_async")?;

        let invocation_query_str = r#"
(invocation_expression
  function: (_) @fn_expr
  arguments: (argument_list) @args) @invocation
"#;
        let invocation_query = Query::new(&csharp_lang(), invocation_query_str)
            .with_context(|| "failed to compile invocation query for sync_blocking_in_async")?;

        Ok(Self {
            method_query: Arc::new(method_query),
            member_access_query: Arc::new(member_access_query),
            invocation_query: Arc::new(invocation_query),
        })
    }

    fn find_async_body_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<Range<usize>> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);
        let mut ranges = Vec::new();

        let method_decl_idx = find_capture_index(&self.method_query, "method_decl");
        let body_idx = find_capture_index(&self.method_query, "method_body");

        while let Some(m) = matches.next() {
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_decl_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let (Some(decl), Some(body)) = (decl_node, body_node)
                && has_modifier(decl, source, "async")
            {
                ranges.push(body.start_byte()..body.end_byte());
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
        "Detects blocking calls (.Result, .Wait(), Thread.Sleep) inside async methods"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let async_ranges = self.find_async_body_ranges(tree, source);
        if async_ranges.is_empty() {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Pattern 1: .Result property access inside async method
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.member_access_query, tree.root_node(), source);

            let member_name_idx = find_capture_index(&self.member_access_query, "member_name");
            let member_access_idx = find_capture_index(&self.member_access_query, "member_access");

            while let Some(m) = matches.next() {
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == member_name_idx)
                    .map(|c| c.node);
                let access_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == member_access_idx)
                    .map(|c| c.node);

                if let (Some(name_node), Some(access_node)) = (name_node, access_node) {
                    let name = node_text(name_node, source);
                    if name == "Result"
                        && Self::is_in_async_body(&async_ranges, access_node.start_byte())
                    {
                        let start = access_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "task_result_in_async".to_string(),
                            message: "accessing `.Result` inside async method blocks the thread and risks deadlocks \u{2014} use `await` instead".to_string(),
                            snippet: extract_snippet(source, access_node, 1),
                        });
                    }
                }
            }
        }

        // Pattern 2: .Wait(), .GetAwaiter().GetResult(), Task.WaitAll(), Task.WaitAny() calls
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

            let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
            let inv_idx = find_capture_index(&self.invocation_query, "invocation");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_idx)
                    .map(|c| c.node);
                let inv_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == inv_idx)
                    .map(|c| c.node);

                if let (Some(fn_node), Some(inv_node)) = (fn_node, inv_node) {
                    if !Self::is_in_async_body(&async_ranges, inv_node.start_byte()) {
                        continue;
                    }

                    let fn_text = node_text(fn_node, source);

                    // .Wait()
                    if fn_text.ends_with(".Wait") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "task_wait_in_async".to_string(),
                            message: "`.Wait()` inside async method blocks the thread and risks deadlocks \u{2014} use `await` instead".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                        continue;
                    }

                    // .GetAwaiter().GetResult()
                    if fn_text.ends_with(".GetResult") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "task_result_in_async".to_string(),
                            message: "`.GetAwaiter().GetResult()` inside async method blocks the thread and risks deadlocks \u{2014} use `await` instead".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                        continue;
                    }

                    // Thread.Sleep()
                    if fn_text == "Thread.Sleep" || fn_text.ends_with(".Thread.Sleep") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "thread_sleep_in_async".to_string(),
                            message: "`Thread.Sleep()` inside async method blocks the thread \u{2014} use `await Task.Delay()` instead".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                        continue;
                    }

                    // Task.WaitAll()
                    if fn_text == "Task.WaitAll" || fn_text.ends_with(".Task.WaitAll") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "task_wait_in_async".to_string(),
                            message: "`Task.WaitAll()` inside async method blocks the thread \u{2014} use `await Task.WhenAll()` instead".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                        continue;
                    }

                    // Task.WaitAny()
                    if fn_text == "Task.WaitAny" || fn_text.ends_with(".Task.WaitAny") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "task_wait_in_async".to_string(),
                            message: "`Task.WaitAny()` inside async method blocks the thread \u{2014} use `await Task.WhenAny()` instead".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_task_result_in_async() {
        let src = r#"
class Foo {
    async Task Bar() {
        var result = someTask.Result;
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "task_result_in_async")
            .collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].severity, "error");
    }

    #[test]
    fn detects_wait_in_async() {
        let src = r#"
class Foo {
    async Task Bar() {
        someTask.Wait();
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "task_wait_in_async")
            .collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].severity, "error");
    }

    #[test]
    fn detects_get_result_in_async() {
        let src = r#"
class Foo {
    async Task Bar() {
        var x = someTask.GetAwaiter().GetResult();
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "task_result_in_async")
            .collect();
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn detects_thread_sleep_in_async() {
        let src = r#"
class Foo {
    async Task Bar() {
        Thread.Sleep(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "thread_sleep_in_async")
            .collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].severity, "warning");
    }

    #[test]
    fn detects_task_wait_all_in_async() {
        let src = r#"
class Foo {
    async Task Bar() {
        Task.WaitAll(task1, task2);
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "task_wait_in_async")
            .collect();
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn detects_task_wait_any_in_async() {
        let src = r#"
class Foo {
    async Task Bar() {
        Task.WaitAny(task1, task2);
    }
}
"#;
        let findings = parse_and_check(src);
        let matched: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "task_wait_in_async")
            .collect();
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn ignores_blocking_in_sync_method() {
        let src = r#"
class Foo {
    void Bar() {
        var result = someTask.Result;
        someTask.Wait();
        Thread.Sleep(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_proper_await_in_async() {
        let src = r#"
class Foo {
    async Task Bar() {
        var result = await someTask;
        await Task.Delay(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
