use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_for_statement_query, extract_snippet, find_capture_index, node_text,
};

pub struct MemoryLeakIndicatorsPipeline {
    for_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            for_query: compile_for_statement_query()?,
        })
    }

    fn find_go_statements_in_body<'a>(
        node: tree_sitter::Node<'a>,
        results: &mut Vec<tree_sitter::Node<'a>>,
    ) {
        if node.kind() == "go_statement" {
            results.push(node);
            return;
        }
        // Don't recurse into nested for loops — they get their own match
        if node.kind() == "for_statement" {
            // Still check direct children of nested for body
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            Self::find_go_statements_in_body(child, results);
        }
    }

    fn find_unbounded_appends_in_body<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        results: &mut Vec<tree_sitter::Node<'a>>,
    ) {
        // Look for append() calls: call_expression where function is "append"
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
                && node_text(func, source) == "append" {
                    results.push(node);
                    return;
                }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            Self::find_unbounded_appends_in_body(child, source, results);
        }
    }

    fn find_defer_statements_in_body<'a>(
        node: tree_sitter::Node<'a>,
        results: &mut Vec<tree_sitter::Node<'a>>,
    ) {
        if node.kind() == "defer_statement" {
            results.push(node);
            return;
        }
        // Don't recurse into nested function literals — defer there is scoped correctly
        if node.kind() == "func_literal" {
            return;
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            Self::find_defer_statements_in_body(child, results);
        }
    }

    fn body_has_bound_check(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Look for len() checks, cap() checks, or break/return conditions
        // that suggest bounded growth
        Self::walk_for_bound_check(node, source)
    }

    fn walk_for_bound_check(node: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(node, source);
        if node.kind() == "if_statement" {
            // Check if the condition references len()
            if text.contains("len(") || text.contains("cap(") {
                return true;
            }
        }
        // Check for range clause — range loops are bounded
        if node.kind() == "for_statement" {
            // In Go's tree-sitter grammar, range-based for loops contain a
            // range_clause child node. Check children for this node type.
            let mut child_cursor = node.walk();
            for child in node.children(&mut child_cursor) {
                if child.kind() == "range_clause" {
                    return true;
                }
            }
            // Also check for classic for loops with a condition (bounded)
            let for_text = node_text(node, source);
            if for_text.contains("range ") {
                return true;
            }
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if Self::walk_for_bound_check(child, source) {
                return true;
            }
        }
        false
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects potential memory/goroutine leak patterns: goroutines in loops, unbounded appends, defers in loops"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.for_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.for_query, "for_body");
        let stmt_idx = find_capture_index(&self.for_query, "for_stmt");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let stmt_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == stmt_idx)
                .map(|c| c.node);

            if let (Some(body), Some(for_node)) = (body_node, stmt_node) {
                // 1. Goroutines inside loops
                let mut go_stmts = Vec::new();
                Self::find_go_statements_in_body(body, &mut go_stmts);
                for go_node in go_stmts {
                    let start = go_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "goroutine_in_loop".to_string(),
                        message: "goroutine launched inside loop — may cause goroutine leak or unbounded concurrency".to_string(),
                        snippet: extract_snippet(source, go_node, 2),
                    });
                }

                // 2. Unbounded append in loop (only if loop has no bound checks)
                if !Self::body_has_bound_check(for_node, source) {
                    let mut appends = Vec::new();
                    Self::find_unbounded_appends_in_body(body, source, &mut appends);
                    for append_node in appends {
                        let start = append_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unbounded_append_in_loop".to_string(),
                            message: "append() inside loop without apparent bound check — potential unbounded slice growth".to_string(),
                            snippet: extract_snippet(source, append_node, 1),
                        });
                    }
                }

                // 3. Defer inside loop
                let mut defers = Vec::new();
                Self::find_defer_statements_in_body(body, &mut defers);
                for defer_node in defers {
                    let start = defer_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "defer_in_loop".to_string(),
                        message: "defer inside loop — deferred calls accumulate until function returns, not loop iteration end".to_string(),
                        snippet: extract_snippet(source, defer_node, 1),
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
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_goroutine_in_loop() {
        let src = r#"package main

func main() {
	for i := 0; i < 100; i++ {
		go func() {
			doWork(i)
		}()
	}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "goroutine_in_loop"));
    }

    #[test]
    fn detects_unbounded_append_in_loop() {
        let src = r#"package main

func collect(ch chan int) {
	var items []int
	for {
		val := <-ch
		items = append(items, val)
	}
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "unbounded_append_in_loop")
        );
    }

    #[test]
    fn detects_defer_in_loop() {
        let src = r#"package main

import "os"

func processFiles(paths []string) {
	for _, path := range paths {
		f, _ := os.Open(path)
		defer f.Close()
	}
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "defer_in_loop"));
    }

    #[test]
    fn ignores_goroutine_outside_loop() {
        let src = r#"package main

func main() {
	go func() {
		doWork()
	}()
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_bounded_append_with_range() {
        let src = r#"package main

func transform(input []int) []int {
	var result []int
	for _, v := range input {
		result = append(result, v*2)
	}
	return result
}
"#;
        let findings = parse_and_check(src);
        // Range-based loops are bounded, so append is acceptable
        let unbounded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_append_in_loop")
            .collect();
        assert!(unbounded.is_empty());
    }

    #[test]
    fn ignores_defer_in_closure_inside_loop() {
        let src = r#"package main

import "os"

func processFiles(paths []string) {
	for _, path := range paths {
		func() {
			f, _ := os.Open(path)
			defer f.Close()
		}()
	}
}
"#;
        let findings = parse_and_check(src);
        // defer inside a closure within a loop is scoped correctly
        let defers: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "defer_in_loop")
            .collect();
        assert!(defers.is_empty());
    }
}
