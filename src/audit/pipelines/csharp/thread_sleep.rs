use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::csharp_primitives::{compile_invocation_query, extract_snippet, find_capture_index, node_text};

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

impl Pipeline for ThreadSleepPipeline {
    fn name(&self) -> &str {
        "thread_sleep"
    }

    fn description(&self) -> &str {
        "Detects Thread.Sleep() calls which block the current thread"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.invocation_query, "fn_expr");
        let invocation_idx = find_capture_index(&self.invocation_query, "invocation");

        while let Some(m) = matches.next() {
            let fn_node = m.captures.iter().find(|c| c.index as usize == fn_expr_idx).map(|c| c.node);
            let inv_node = m.captures.iter().find(|c| c.index as usize == invocation_idx).map(|c| c.node);

            if let (Some(fn_node), Some(inv_node)) = (fn_node, inv_node) {
                let fn_text = node_text(fn_node, source);
                if fn_text == "Thread.Sleep" || fn_text.ends_with(".Thread.Sleep") {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "thread_sleep_call".to_string(),
                        message: "Thread.Sleep() blocks the current thread \u{2014} use Task.Delay() with async/await instead".to_string(),
                        snippet: extract_snippet(source, inv_node, 3),
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
        parser.set_language(&Language::CSharp.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ThreadSleepPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
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
}
