use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_generated_go_file, is_nolint_suppressed};

use super::primitives::{
    compile_go_statement_query, extract_snippet, find_capture_index,
};

pub struct GoroutineLeakPipeline {
    go_query: Arc<Query>,
}

impl GoroutineLeakPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            go_query: compile_go_statement_query()?,
        })
    }

    fn is_for_range(node: tree_sitter::Node) -> bool {
        // Check if a for_statement has a range_clause child (idiomatic `for range ch` pattern)
        if node.kind() == "for_statement" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "range_clause" {
                    return true;
                }
            }
        }
        false
    }

    fn has_for_loop(node: tree_sitter::Node) -> bool {
        if node.kind() == "for_statement" && !Self::is_for_range(node) {
            return true;
        }
        // for_range_statement is a separate node kind in some Go grammars
        if node.kind() == "for_range_statement" {
            return false;
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if Self::has_for_loop(child) {
                return true;
            }
        }
        false
    }

    fn for_loop_has_select_done(node: tree_sitter::Node) -> bool {
        if node.kind() == "for_statement" {
            return Self::contains_select_done(node);
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if child.kind() == "for_statement" && Self::contains_select_done(child) {
                return true;
            }
            if Self::for_loop_has_select_done(child) {
                return true;
            }
        }
        false
    }

    fn contains_select_done(for_node: tree_sitter::Node) -> bool {
        // Walk descendants looking for select_statement containing a receive
        Self::walk_for_select(for_node)
    }

    fn has_receive_in_select(node: tree_sitter::Node) -> bool {
        if node.kind() == "receive_statement" {
            return true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::has_receive_in_select(child) {
                return true;
            }
        }
        false
    }

    fn walk_for_select(node: tree_sitter::Node) -> bool {
        if node.kind() == "select_statement" {
            // Check if any communication_case has a receive_statement descendant
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "communication_case" && Self::has_receive_in_select(child) {
                    return true;
                }
            }
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if Self::walk_for_select(child) {
                return true;
            }
        }
        false
    }
}

impl GraphPipeline for GoroutineLeakPipeline {
    fn name(&self) -> &str {
        "goroutine_leak"
    }

    fn description(&self) -> &str {
        "Detects `go func()` with for-loop but no select+ctx.Done() — potential goroutine leak"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_generated_go_file(file_path, source) {
            return vec![];
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.go_query, tree.root_node(), source);

        let expr_idx = find_capture_index(&self.go_query, "go_expr");
        let stmt_idx = find_capture_index(&self.go_query, "go_stmt");

        while let Some(m) = matches.next() {
            let expr_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == expr_idx)
                .map(|c| c.node);
            let stmt_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == stmt_idx)
                .map(|c| c.node);

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

                if Self::for_loop_has_select_done(body) {
                    continue;
                }

                if is_nolint_suppressed(source, stmt_node, self.name()) {
                    continue;
                }

                let start = stmt_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "goroutine_missing_done_channel".to_string(),
                    message: "goroutine with for-loop but no select+ctx.Done() — may leak"
                        .to_string(),
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
        let pipeline = GoroutineLeakPipeline::new().unwrap();
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
    fn detects_goroutine_with_for_no_done() {
        let src = r#"package main
func main() {
	go func() {
		for {
			doWork()
		}
	}()
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "goroutine_missing_done_channel");
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

    #[test]
    fn for_range_channel_not_flagged() {
        let src = r#"package main
func worker() {
    go func() {
        for item := range ch {
            process(item)
        }
    }()
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression_skips_finding() {
        let src = "package main\nfunc worker() {\n\tgo func() { // NOLINT(goroutine_leak)\n\t\tfor {\n\t\t\tdoWork()\n\t\t}\n\t}()\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc worker() {\n\tgo func() {\n\t\tfor {\n\t\t\tdoWork()\n\t\t}\n\t}()\n}\n";
        let findings = parse_and_check_file(src, "worker.pb.go");
        assert!(findings.is_empty());
    }
}
