use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_invocation_query, compile_member_access_query, compile_method_decl_query,
    extract_snippet, find_capture_index, has_modifier, node_text,
};

pub struct SyncOverAsyncPipeline {
    member_access_query: Arc<Query>,
    invocation_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl SyncOverAsyncPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            member_access_query: compile_member_access_query()?,
            invocation_query: compile_invocation_query()?,
            method_query: compile_method_decl_query()?,
        })
    }
}

impl Pipeline for SyncOverAsyncPipeline {
    fn name(&self) -> &str {
        "sync_over_async"
    }

    fn description(&self) -> &str {
        "Detects blocking calls on async code (.Result, .Wait()) and async void methods"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Pattern 1: blocking .Result access
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

                if let (Some(name_node), Some(access_node)) = (name_node, access_node)
                    && node_text(name_node, source) == "Result"
                {
                    let start = access_node.start_position();
                    findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "blocking_result_access".to_string(),
                            message: "accessing `.Result` blocks the calling thread \u{2014} use `await` instead".to_string(),
                            snippet: extract_snippet(source, access_node, 3),
                        });
                }
            }
        }

        // Pattern 2: blocking .Wait() call
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);
            let fn_expr_idx = find_capture_index(&self.invocation_query, "fn_expr");
            let invocation_idx = find_capture_index(&self.invocation_query, "invocation");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_expr_idx)
                    .map(|c| c.node);
                let inv_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == invocation_idx)
                    .map(|c| c.node);

                if let (Some(fn_node), Some(inv_node)) = (fn_node, inv_node) {
                    let fn_text = node_text(fn_node, source);
                    if fn_text.ends_with(".Wait")
                        || fn_text.ends_with(".WaitAll")
                        || fn_text.ends_with(".WaitAny")
                    {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "blocking_wait_call".to_string(),
                            message:
                                "`.Wait()` blocks the calling thread \u{2014} use `await` instead"
                                    .to_string(),
                            snippet: extract_snippet(source, inv_node, 3),
                        });
                    }
                }
            }
        }

        // Pattern 3: async void methods
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);
            let return_type_idx = find_capture_index(&self.method_query, "return_type");
            let method_name_idx = find_capture_index(&self.method_query, "method_name");
            let method_decl_idx = find_capture_index(&self.method_query, "method_decl");

            while let Some(m) = matches.next() {
                let type_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == return_type_idx)
                    .map(|c| c.node);
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_name_idx)
                    .map(|c| c.node);
                let decl_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_decl_idx)
                    .map(|c| c.node);

                if let (Some(type_node), Some(name_node), Some(decl_node)) =
                    (type_node, name_node, decl_node)
                    && has_modifier(decl_node, source, "async")
                    && node_text(type_node, source) == "void"
                {
                    let method_name = node_text(name_node, source);
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "async_void".to_string(),
                            message: format!(
                                "`async void {method_name}` cannot be awaited and exceptions crash the process \u{2014} use `async Task` instead"
                            ),
                            snippet: extract_snippet(source, decl_node, 3),
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SyncOverAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_blocking_result() {
        let src = r#"
class Foo {
    void Bar() {
        var result = someTask.Result;
    }
}
"#;
        let findings = parse_and_check(src);
        let result_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "blocking_result_access")
            .collect();
        assert_eq!(result_findings.len(), 1);
    }

    #[test]
    fn detects_blocking_wait() {
        let src = r#"
class Foo {
    void Bar() {
        someTask.Wait();
    }
}
"#;
        let findings = parse_and_check(src);
        let wait_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "blocking_wait_call")
            .collect();
        assert_eq!(wait_findings.len(), 1);
    }

    #[test]
    fn detects_async_void() {
        let src = r#"
class Foo {
    async void OnClick() {
        await DoWork();
    }
}
"#;
        let findings = parse_and_check(src);
        let async_void: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "async_void")
            .collect();
        assert_eq!(async_void.len(), 1);
    }

    #[test]
    fn clean_async_task() {
        let src = r#"
class Foo {
    async Task DoWork() {
        await Task.Delay(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_await_usage() {
        let src = r#"
class Foo {
    async Task Bar() {
        var result = await someTask;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
