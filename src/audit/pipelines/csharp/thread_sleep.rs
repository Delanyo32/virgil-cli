use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_invocation_query, extract_snippet, find_capture_index, is_csharp_suppressed, node_text,
};

pub struct ThreadSleepPipeline {
    invocation_query: Arc<Query>,
}

impl ThreadSleepPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            invocation_query: compile_invocation_query()?,
        })
    }
}

/// Check if a node is inside a method with `async` modifier.
fn is_in_async_method(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "method_declaration" || n.kind() == "local_function_statement" {
            // Check for async modifier
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if child.kind() == "modifier" && child.utf8_text(source).unwrap_or("") == "async" {
                    return true;
                }
            }
            return false;
        }
        current = n.parent();
    }
    false
}

/// Extract the first argument text from an invocation's argument list node.
fn get_first_arg_text<'a>(args_node: tree_sitter::Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        if child.kind() == "argument" {
            if let Some(expr) = child.named_child(0) {
                return expr.utf8_text(source).ok();
            }
        }
    }
    None
}

impl GraphPipeline for ThreadSleepPipeline {
    fn name(&self) -> &str {
        "thread_sleep"
    }

    fn description(&self) -> &str {
        "Detects Thread.Sleep() calls which block the current thread"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.invocation_query, "fn_expr");
        let invocation_idx = find_capture_index(&self.invocation_query, "invocation");
        let args_idx = find_capture_index(&self.invocation_query, "args");

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
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(inv_node)) = (fn_node, inv_node) {
                let fn_text = node_text(fn_node, source);

                let is_thread_sleep = fn_text == "Thread.Sleep"
                    || fn_text.ends_with(".Thread.Sleep")
                    || fn_text == "System.Threading.Thread.Sleep";

                if !is_thread_sleep {
                    continue;
                }

                // Check suppression
                if is_csharp_suppressed(source, inv_node, self.name()) {
                    continue;
                }

                // Check for Thread.Sleep(0) or Thread.Sleep(1) — yield idioms
                if let Some(args) = args_node {
                    if let Some(arg_text) = get_first_arg_text(args, source) {
                        if arg_text == "0" || arg_text == "1" {
                            continue;
                        }
                    }
                }

                // Severity graduation: async context → error, sync → warning
                let severity = if is_in_async_method(inv_node, source) {
                    "error"
                } else {
                    "warning"
                };

                let start = inv_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "thread_sleep_call".to_string(),
                    message: "Thread.Sleep() blocks the current thread \u{2014} use Task.Delay() with async/await instead".to_string(),
                    snippet: extract_snippet(source, inv_node, 3),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "Service.cs")
    }

    fn parse_and_check_with_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ThreadSleepPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_thread_sleep() {
        let src = r#"
class Foo {
    void Bar() {
        Thread.Sleep(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "thread_sleep_call");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn clean_task_delay() {
        let src = r#"
class Foo {
    async Task Bar() {
        await Task.Delay(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_unrelated_invocations() {
        let src = r#"
class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_fully_qualified() {
        let src = r#"
class Foo {
    void Bar() {
        System.Threading.Thread.Sleep(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "thread_sleep_call");
    }

    #[test]
    fn skips_sleep_zero_yield() {
        let src = r#"
class Foo {
    void Bar() {
        Thread.Sleep(0);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_sleep_one_yield() {
        let src = r#"
class Foo {
    void Bar() {
        Thread.Sleep(1);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn error_severity_in_async_method() {
        let src = r#"
class Foo {
    async Task Bar() {
        Thread.Sleep(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn warning_severity_in_sync_method() {
        let src = r#"
class Foo {
    void Bar() {
        Thread.Sleep(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn excluded_in_test_files() {
        let src = r#"
class FooTests {
    void TestBar() {
        Thread.Sleep(500);
    }
}
"#;
        let findings = parse_and_check_with_path(src, "FooTests.cs");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppressed_by_nolint() {
        let src = r#"
class Foo {
    void Bar() {
        // NOLINT
        Thread.Sleep(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
