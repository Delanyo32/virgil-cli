use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_nolint_suppressed, is_generated_go_file};

use super::primitives::{
    compile_method_call_query, extract_snippet, find_capture_index, node_text,
};

pub struct MutexMisusePipeline {
    call_query: Arc<Query>,
}

impl MutexMisusePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_method_call_query()?,
        })
    }

    /// Find the enclosing function body for a given node.
    fn enclosing_function_body(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        let mut current = node.parent();
        while let Some(parent) = current {
            let kind = parent.kind();
            if kind == "function_declaration"
                || kind == "method_declaration"
                || kind == "func_literal"
            {
                return parent.child_by_field_name("body");
            }
            current = parent.parent();
        }
        None
    }

    /// Extract the receiver (operand) text from a Lock/RLock call node.
    fn lock_receiver_text<'a>(
        call_node: tree_sitter::Node<'a>,
        source: &'a [u8],
    ) -> Option<&'a str> {
        let func_node = call_node.child_by_field_name("function")?;
        if func_node.kind() == "selector_expression" {
            let operand = func_node.child_by_field_name("operand")?;
            return operand.utf8_text(source).ok();
        }
        None
    }

    /// Check if Unlock() (or RUnlock()) is called on the same receiver anywhere in the given body node.
    fn has_unlock_in_scope(
        body: tree_sitter::Node,
        source: &[u8],
        receiver: &str,
        expected_unlock: &str,
    ) -> bool {
        Self::walk_for_unlock(body, source, receiver, expected_unlock)
    }

    fn walk_for_unlock(
        node: tree_sitter::Node,
        source: &[u8],
        receiver: &str,
        expected_unlock: &str,
    ) -> bool {
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
            && func.kind() == "selector_expression"
        {
            let operand = func
                .child_by_field_name("operand")
                .and_then(|o| o.utf8_text(source).ok())
                .unwrap_or("");
            let method = func
                .child_by_field_name("field")
                .and_then(|f| f.utf8_text(source).ok())
                .unwrap_or("");
            if operand == receiver && method == expected_unlock {
                return true;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::walk_for_unlock(child, source, receiver, expected_unlock) {
                return true;
            }
        }
        false
    }
}

impl GraphPipeline for MutexMisusePipeline {
    fn name(&self) -> &str {
        "mutex_misuse"
    }

    fn description(&self) -> &str {
        "Detects .Lock() not immediately followed by defer .Unlock()"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        if is_generated_go_file(file_path, source) {
            return vec![];
        }
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.call_query, "method_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(method_node), Some(call_node)) = (method_node, call_node) {
                if is_nolint_suppressed(source, call_node, self.name()) {
                    continue;
                }
                let method_name = node_text(method_node, source);
                if method_name != "Lock" && method_name != "RLock" && method_name != "TryLock" {
                    continue;
                }

                let expected_unlock = if method_name == "RLock" {
                    "RUnlock"
                } else {
                    "Unlock" // covers both Lock and TryLock
                };

                // Extract receiver of the Lock call
                let receiver = Self::lock_receiver_text(call_node, source).unwrap_or("");

                // Check if Unlock exists anywhere in the enclosing function body
                let has_unlock = if let Some(body) = Self::enclosing_function_body(call_node) {
                    Self::has_unlock_in_scope(body, source, receiver, expected_unlock)
                } else {
                    false
                };

                if !has_unlock {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "lock_without_defer_unlock".to_string(),
                        message: format!(
                            ".{method_name}() without .{expected_unlock}() in function scope — risk of deadlock"
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_file(source, "test.go")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MutexMisusePipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
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
    fn detects_lock_without_any_unlock() {
        let src = "package main\nfunc doWork() {\n\tmu.Lock()\n\tfmt.Println(\"done\")\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "lock_without_defer_unlock");
    }

    #[test]
    fn clean_lock_with_unlock_later() {
        // Unlock() exists in function scope (not deferred but present)
        let src = "package main\nfunc doWork() {\n\tmu.Lock()\n\tmu.Unlock()\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
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

    #[test]
    fn nolint_suppression_skips_finding() {
        let src = "package main\nfunc doWork() {\n\tmu.Lock() // NOLINT(mutex_misuse)\n\tfmt.Println(\"done\")\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc doWork() {\n\tmu.Lock()\n\tfmt.Println(\"done\")\n}\n";
        let findings = parse_and_check_file(src, "model.pb.go");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_trylock_without_unlock() {
        let src = "package main\nfunc doWork() {\n\tmu.TryLock()\n\tfmt.Println(\"done\")\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("TryLock"));
    }

    #[test]
    fn detects_rlock_without_runlock() {
        let src = "package main\nfunc doWork() {\n\tmu.RLock()\n\tfmt.Println(\"done\")\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("RLock"));
    }
}
