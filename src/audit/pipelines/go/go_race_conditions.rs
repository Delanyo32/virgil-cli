use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_for_statement_query, compile_go_statement_query, extract_snippet, find_capture_index,
    node_text,
};

pub struct GoRaceConditionsPipeline {
    go_query: Arc<Query>,
    for_query: Arc<Query>,
}

impl GoRaceConditionsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            go_query: compile_go_statement_query()?,
            for_query: compile_for_statement_query()?,
        })
    }

    fn contains_go_statement(node: tree_sitter::Node) -> bool {
        if node.kind() == "go_statement" {
            return true;
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if Self::contains_go_statement(child) {
                return true;
            }
        }
        false
    }

    fn contains_bracket_access(node: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(node, source);
        text.contains('[')
    }
}

impl Pipeline for GoRaceConditionsPipeline {
    fn name(&self) -> &str {
        "race_conditions"
    }

    fn description(&self) -> &str {
        "Detects race condition risks: map access in goroutines, loop variable capture"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // 1. Detect goroutines spawned inside for loops (loop variable capture)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.for_query, tree.root_node(), source);

            let for_body_idx = find_capture_index(&self.for_query, "for_body");
            let for_stmt_idx = find_capture_index(&self.for_query, "for_stmt");

            while let Some(m) = matches.next() {
                let body_node = m.captures.iter().find(|c| c.index as usize == for_body_idx).map(|c| c.node);
                let stmt_node = m.captures.iter().find(|c| c.index as usize == for_stmt_idx).map(|c| c.node);

                if let (Some(body), Some(stmt)) = (body_node, stmt_node) {
                    if Self::contains_go_statement(body) {
                        let start = stmt.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "loop_var_capture".to_string(),
                            message: "goroutine spawned inside for loop — loop variable may be captured by reference".to_string(),
                            snippet: extract_snippet(source, stmt, 3),
                        });
                    }
                }
            }
        }

        // 2. Detect goroutines with map-like bracket access (map race)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.go_query, tree.root_node(), source);

            let go_expr_idx = find_capture_index(&self.go_query, "go_expr");
            let go_stmt_idx = find_capture_index(&self.go_query, "go_stmt");

            while let Some(m) = matches.next() {
                let expr_node = m.captures.iter().find(|c| c.index as usize == go_expr_idx).map(|c| c.node);
                let stmt_node = m.captures.iter().find(|c| c.index as usize == go_stmt_idx).map(|c| c.node);

                if let (Some(expr), Some(stmt)) = (expr_node, stmt_node) {
                    if Self::contains_bracket_access(expr, source) {
                        let start = stmt.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "map_race".to_string(),
                            message: "goroutine body contains map/slice index access — concurrent map access without synchronization causes data race".to_string(),
                            snippet: extract_snippet(source, stmt, 3),
                        });
                    }
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
        let pipeline = GoRaceConditionsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_loop_var_capture() {
        let src = r#"package main

func main() {
	for i := 0; i < 10; i++ {
		go func() {
			fmt.Println(i)
		}()
	}
}
"#;
        let findings = parse_and_check(src);
        let loop_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "loop_var_capture")
            .collect();
        assert_eq!(loop_findings.len(), 1);
    }

    #[test]
    fn detects_goroutine_in_for() {
        let src = r#"package main

func process(items []string) {
	for _, item := range items {
		go func() {
			handle(item)
		}()
	}
}
"#;
        let findings = parse_and_check(src);
        let loop_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "loop_var_capture")
            .collect();
        assert!(!loop_findings.is_empty());
    }

    #[test]
    fn clean_no_findings() {
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
