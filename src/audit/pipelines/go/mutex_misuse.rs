use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::go_primitives::{compile_method_call_query, extract_snippet, find_capture_index, node_text};

pub struct MutexMisusePipeline {
    call_query: Arc<Query>,
}

impl MutexMisusePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_method_call_query()?,
        })
    }

    fn is_defer_unlock_sibling(expr_stmt: tree_sitter::Node, source: &[u8]) -> bool {
        if let Some(next) = expr_stmt.next_named_sibling() {
            if next.kind() == "defer_statement" {
                // Check if the defer contains .Unlock()
                let text = node_text(next, source);
                return text.contains("Unlock()");
            }
        }
        false
    }
}

impl Pipeline for MutexMisusePipeline {
    fn name(&self) -> &str {
        "mutex_misuse"
    }

    fn description(&self) -> &str {
        "Detects .Lock() not immediately followed by defer .Unlock()"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.call_query, "method_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx).map(|c| c.node);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx).map(|c| c.node);

            if let (Some(method_node), Some(call_node)) = (method_node, call_node) {
                let method_name = node_text(method_node, source);
                if method_name != "Lock" && method_name != "RLock" {
                    continue;
                }

                // Find the parent expression_statement
                let expr_stmt = if let Some(parent) = call_node.parent() {
                    if parent.kind() == "expression_statement" {
                        parent
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                if !Self::is_defer_unlock_sibling(expr_stmt, source) {
                    let start = call_node.start_position();
                    let expected_unlock = if method_name == "RLock" {
                        "RUnlock"
                    } else {
                        "Unlock"
                    };
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "lock_without_defer_unlock".to_string(),
                        message: format!(
                            ".{method_name}() without immediate `defer .{expected_unlock}()` — risk of deadlock"
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MutexMisusePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_lock_without_defer() {
        let src = "package main\nfunc doWork() {\n\tmu.Lock()\n\tmu.Unlock()\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "lock_without_defer_unlock");
    }

    #[test]
    fn clean_lock_with_defer() {
        let src = "package main\nfunc doWork() {\n\tmu.Lock()\n\tdefer mu.Unlock()\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_lock_followed_by_wrong_defer() {
        let src = "package main\nfunc doWork() {\n\tmu.Lock()\n\tdefer fmt.Println(\"done\")\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
