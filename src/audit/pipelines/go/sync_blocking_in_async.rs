use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_go_statement_query, extract_snippet, find_capture_index, node_text,
};

const BLOCKING_CALLS: &[(&str, &str)] = &[
    ("time", "Sleep"),
    ("http", "Get"),
    ("http", "Post"),
    ("http", "Head"),
    ("net", "Dial"),
    ("net", "DialTimeout"),
    ("os", "Open"),
    ("os", "ReadFile"),
    ("os", "WriteFile"),
    ("ioutil", "ReadAll"),
    ("ioutil", "ReadFile"),
    ("io", "ReadAll"),
    ("io", "Copy"),
];

pub struct SyncBlockingInAsyncPipeline {
    go_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            go_query: compile_go_statement_query()?,
        })
    }

    fn find_blocking_in_body<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        findings: &mut Vec<(tree_sitter::Node<'a>, String)>,
    ) {
        // Check selector_expression calls (pkg.Method)
        if node.kind() == "call_expression" {
            if let Some(func) = node.child_by_field_name("function") {
                if func.kind() == "selector_expression" {
                    if let Some(operand) = func.child_by_field_name("operand") {
                        if let Some(field) = func.child_by_field_name("field") {
                            let pkg_name = node_text(operand, source);
                            let method_name = node_text(field, source);

                            for (pkg, method) in BLOCKING_CALLS {
                                if pkg_name == *pkg && method_name == *method {
                                    findings.push((node, format!("{pkg_name}.{method_name}")));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check for bare channel receive without select (<-ch)
        if node.kind() == "unary_expression" {
            let text = node_text(node, source);
            if text.starts_with("<-") {
                // Check if this receive is inside a select — if so, it's fine
                if !Self::is_inside_select(node) {
                    findings.push((node, "bare channel receive".to_string()));
                }
            }
        }

        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            Self::find_blocking_in_body(child, source, findings);
        }
    }

    fn is_inside_select(node: tree_sitter::Node) -> bool {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "select_statement" {
                return true;
            }
            // Stop at function boundary
            if parent.kind() == "func_literal" || parent.kind() == "function_declaration" {
                return false;
            }
            current = parent;
        }
        false
    }
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects blocking calls inside goroutines that may stall concurrent execution"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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
                // Look for func_literal body in the go expression
                let func_node = if expr_node.kind() == "call_expression" {
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

                let mut blocking_calls = Vec::new();
                Self::find_blocking_in_body(body, source, &mut blocking_calls);

                for (call_node, call_desc) in blocking_calls {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "blocking_in_goroutine".to_string(),
                        message: format!(
                            "blocking call `{call_desc}` inside goroutine may stall concurrent execution"
                        ),
                        snippet: extract_snippet(source, stmt_node, 3),
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
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_time_sleep_in_goroutine() {
        let src = r#"package main

import "time"

func main() {
	go func() {
		time.Sleep(5 * time.Second)
		doWork()
	}()
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_in_goroutine");
        assert!(findings[0].message.contains("time.Sleep"));
    }

    #[test]
    fn detects_blocking_io_in_goroutine() {
        let src = r#"package main

import "os"

func main() {
	go func() {
		os.Open("/etc/hosts")
	}()
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "blocking_in_goroutine");
        assert!(findings[0].message.contains("os.Open"));
    }

    #[test]
    fn detects_bare_channel_receive_in_goroutine() {
        let src = r#"package main

func main() {
	ch := make(chan int)
	go func() {
		val := <-ch
		process(val)
	}()
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("bare channel receive"))
        );
    }

    #[test]
    fn ignores_channel_receive_in_select() {
        let src = r#"package main

func main() {
	go func() {
		select {
		case val := <-ch:
			process(val)
		case <-ctx.Done():
			return
		}
	}()
}
"#;
        let findings = parse_and_check(src);
        // channel receives inside select are fine
        let bare_receives: Vec<_> = findings
            .iter()
            .filter(|f| f.message.contains("bare channel receive"))
            .collect();
        assert!(bare_receives.is_empty());
    }

    #[test]
    fn ignores_blocking_call_outside_goroutine() {
        let src = r#"package main

import "time"

func main() {
	time.Sleep(5 * time.Second)
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
