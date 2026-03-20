use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

pub struct SyncBlockingInAsyncPipeline {
    method_invocation_query: Arc<Query>,
    synchronized_query: Arc<Query>,
    method_decl_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        let method_invocation_str = r#"
(method_invocation
  object: (_)? @object
  name: (identifier) @method_name
  arguments: (argument_list) @args) @invocation
"#;
        let method_invocation_query = Query::new(&java_lang(), method_invocation_str)
            .with_context(
                || "failed to compile method_invocation query for sync_blocking_in_async",
            )?;

        let synchronized_str = r#"
(synchronized_statement
  body: (block) @sync_body) @sync_stmt
"#;
        let synchronized_query = Query::new(&java_lang(), synchronized_str).with_context(
            || "failed to compile synchronized_statement query for sync_blocking_in_async",
        )?;

        let method_decl_str = r#"
(method_declaration
  name: (identifier) @method_name
  body: (block) @method_body) @method_decl
"#;
        let method_decl_query = Query::new(&java_lang(), method_decl_str).with_context(
            || "failed to compile method_declaration query for sync_blocking_in_async",
        )?;

        Ok(Self {
            method_invocation_query: Arc::new(method_invocation_query),
            synchronized_query: Arc::new(synchronized_query),
            method_decl_query: Arc::new(method_decl_query),
        })
    }
}

/// Check if a node is inside a CompletableFuture lambda (e.g. supplyAsync, thenApply, etc.)
fn is_inside_completable_future(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "lambda_expression" {
            // Check if the lambda is an argument to a CompletableFuture method
            if let Some(arg_list) = p.parent()
                && arg_list.kind() == "argument_list"
                    && let Some(invocation) = arg_list.parent()
                        && invocation.kind() == "method_invocation" {
                            let inv_text = node_text(invocation, source);
                            if inv_text.contains("CompletableFuture")
                                || inv_text.contains("supplyAsync")
                                || inv_text.contains("runAsync")
                                || inv_text.contains("thenApply")
                                || inv_text.contains("thenCompose")
                                || inv_text.contains("thenAccept")
                            {
                                return true;
                            }
                        }
        }
        if p.kind() == "method_declaration" || p.kind() == "class_declaration" {
            break;
        }
        parent = p.parent();
    }
    false
}

/// Check if a method declaration has @Async annotation
fn has_async_annotation(method_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = method_node.walk();
    for child in method_node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mod_cursor = child.walk();
            for modifier in child.children(&mut mod_cursor) {
                if modifier.kind() == "marker_annotation" || modifier.kind() == "annotation" {
                    let text = node_text(modifier, source);
                    if text.contains("Async") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects blocking calls in async contexts: Thread.sleep, blocking .get() on Future, synchronized in async"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // 1. Find Thread.sleep() calls and blocking I/O in async contexts
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.method_invocation_query, tree.root_node(), source);

            let object_idx = find_capture_index(&self.method_invocation_query, "object");
            let method_idx = find_capture_index(&self.method_invocation_query, "method_name");
            let invocation_idx = find_capture_index(&self.method_invocation_query, "invocation");

            while let Some(m) = matches.next() {
                let object_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == object_idx)
                    .map(|c| c.node);
                let method_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_idx)
                    .map(|c| c.node);
                let inv_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == invocation_idx)
                    .map(|c| c.node);

                if let (Some(method_node), Some(inv_node)) = (method_node, inv_node) {
                    let method_name = node_text(method_node, source);

                    // Thread.sleep() detection
                    if method_name == "sleep"
                        && let Some(obj) = object_node {
                            let obj_text = node_text(obj, source);
                            if obj_text == "Thread" {
                                let in_async = is_inside_completable_future(inv_node, source)
                                    || is_inside_async_method(
                                        inv_node,
                                        source,
                                        &self.method_decl_query,
                                    );
                                if in_async {
                                    let start = inv_node.start_position();
                                    findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "warning".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "thread_sleep_in_async".to_string(),
                                        message: "Thread.sleep() in async context — blocks the thread pool, use async delay instead".to_string(),
                                        snippet: extract_snippet(source, inv_node, 2),
                                    });
                                }
                            }
                        }

                    // .get() on Future without timeout (blocking call)
                    if method_name == "get"
                        && let Some(args_node) = m
                            .captures
                            .iter()
                            .find(|c| {
                                c.index as usize
                                    == find_capture_index(&self.method_invocation_query, "args")
                            })
                            .map(|c| c.node)
                        {
                            let args_text = node_text(args_node, source);
                            // .get() with no arguments = blocking without timeout
                            if args_text.trim() == "()" {
                                let start = inv_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "blocking_future_get".to_string(),
                                    message: ".get() on Future without timeout — blocks indefinitely, use .get(timeout, unit) or .join()".to_string(),
                                    snippet: extract_snippet(source, inv_node, 2),
                                });
                            }
                        }
                }
            }
        }

        // 2. Find synchronized blocks inside async contexts
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.synchronized_query, tree.root_node(), source);

            let sync_idx = find_capture_index(&self.synchronized_query, "sync_stmt");

            while let Some(m) = matches.next() {
                let sync_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == sync_idx)
                    .map(|c| c.node);

                if let Some(sync_node) = sync_node
                    && (is_inside_completable_future(sync_node, source)
                        || is_inside_async_method(sync_node, source, &self.method_decl_query))
                    {
                        let start = sync_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "synchronized_in_async".to_string(),
                            message: "synchronized block in async context — can cause thread pool starvation".to_string(),
                            snippet: extract_snippet(source, sync_node, 3),
                        });
                    }
            }
        }

        findings
    }
}

/// Check if a node is inside a method annotated with @Async
fn is_inside_async_method(node: tree_sitter::Node, source: &[u8], _method_query: &Query) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "method_declaration" {
            return has_async_annotation(p, source);
        }
        if p.kind() == "class_declaration" {
            return false;
        }
        parent = p.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_thread_sleep_in_completable_future() {
        let src = r#"class Service {
    void process() {
        CompletableFuture.supplyAsync(() -> {
            Thread.sleep(1000);
            return result;
        });
    }
}"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "thread_sleep_in_async")
        );
    }

    #[test]
    fn detects_blocking_future_get() {
        let src = r#"class Service {
    void process() {
        CompletableFuture<String> future = CompletableFuture.supplyAsync(() -> "result");
        String result = future.get();
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "blocking_future_get"));
    }

    #[test]
    fn ignores_thread_sleep_outside_async() {
        let src = r#"class Service {
    void process() {
        Thread.sleep(1000);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .all(|f| f.pattern != "thread_sleep_in_async")
        );
    }

    #[test]
    fn ignores_future_get_with_timeout() {
        let src = r#"class Service {
    void process() {
        CompletableFuture<String> future = CompletableFuture.supplyAsync(() -> "result");
        String result = future.get(5, TimeUnit.SECONDS);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().all(|f| f.pattern != "blocking_future_get"));
    }

    #[test]
    fn detects_thread_sleep_in_async_method() {
        let src = r#"class Service {
    @Async
    void processAsync() {
        Thread.sleep(1000);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "thread_sleep_in_async")
        );
    }
}
