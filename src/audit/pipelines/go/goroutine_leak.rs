use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::go_primitives::{compile_go_statement_query, extract_snippet, find_capture_index, node_text};

pub struct GoroutineLeakPipeline {
    go_query: Arc<Query>,
}

impl GoroutineLeakPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            go_query: compile_go_statement_query()?,
        })
    }

    fn has_for_loop(node: tree_sitter::Node) -> bool {
        if node.kind() == "for_statement" {
            return true;
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if Self::has_for_loop(child) {
                return true;
            }
        }
        false
    }

    fn for_loop_has_select_done(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "for_statement" {
            return Self::contains_select_done(node, source);
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if child.kind() == "for_statement" && Self::contains_select_done(child, source) {
                return true;
            }
            if Self::for_loop_has_select_done(child, source) {
                return true;
            }
        }
        false
    }

    fn contains_select_done(for_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk descendants looking for select_statement containing .Done()
        Self::walk_for_select(for_node, source)
    }

    fn walk_for_select(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "select_statement" {
            // Check if any communication_case references .Done()
            let text = node_text(node, source);
            if text.contains("Done()") {
                return true;
            }
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if Self::walk_for_select(child, source) {
                return true;
            }
        }
        false
    }
}

impl Pipeline for GoroutineLeakPipeline {
    fn name(&self) -> &str {
        "goroutine_leak"
    }

    fn description(&self) -> &str {
        "Detects `go func()` with for-loop but no select+ctx.Done() — potential goroutine leak"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.go_query, tree.root_node(), source);

        let expr_idx = find_capture_index(&self.go_query, "go_expr");
        let stmt_idx = find_capture_index(&self.go_query, "go_stmt");

        while let Some(m) = matches.next() {
            let expr_node = m.captures.iter().find(|c| c.index as usize == expr_idx).map(|c| c.node);
            let stmt_node = m.captures.iter().find(|c| c.index as usize == stmt_idx).map(|c| c.node);

            if let (Some(expr_node), Some(stmt_node)) = (expr_node, stmt_node) {
                // Look for func_literal in the go expression
                let func_node = if expr_node.kind() == "call_expression" {
                    // go func() { ... }()
                    expr_node.child_by_field_name("function")
                } else {
                    None
                };

                let body_node = func_node.and_then(|f| {
                    if f.kind() == "func_literal" {
                        f.child_by_field_name("body")
                    } else {
                        None
                    }
                });

                let body = match body_node {
                    Some(b) => b,
                    None => continue,
                };

                if !Self::has_for_loop(body) {
                    continue;
                }

                if Self::for_loop_has_select_done(body, source) {
                    continue;
                }

                let start = stmt_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "goroutine_leak".to_string(),
                    message: "goroutine with for-loop but no select+ctx.Done() — may leak".to_string(),
                    snippet: extract_snippet(source, stmt_node, 3),
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GoroutineLeakPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_goroutine_with_for_no_done() {
        let src = r#"package main
func main() {
	go func() {
		for job := range ch {
			process(job)
		}
	}()
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "goroutine_leak");
    }

    #[test]
    fn clean_goroutine_with_select_done() {
        let src = r#"package main
func main() {
	go func() {
		for {
			select {
			case <-ctx.Done():
				return
			case job := <-ch:
				process(job)
			}
		}
	}()
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_goroutine_without_for_loop() {
        let src = r#"package main
func main() {
	go func() {
		doSomething()
	}()
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
