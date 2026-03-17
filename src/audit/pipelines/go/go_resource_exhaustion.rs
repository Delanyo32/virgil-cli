use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{self, extract_snippet, find_capture_index, node_text};

pub struct GoResourceExhaustionPipeline {
    for_query: Arc<Query>,
    go_query: Arc<Query>,
    selector_query: Arc<Query>,
}

impl GoResourceExhaustionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            for_query: primitives::compile_for_statement_query()?,
            go_query: primitives::compile_go_statement_query()?,
            selector_query: primitives::compile_selector_call_query()?,
        })
    }
}

impl Pipeline for GoResourceExhaustionPipeline {
    fn name(&self) -> &str {
        "resource_exhaustion"
    }

    fn description(&self) -> &str {
        "Detects resource exhaustion risks: unbounded goroutine spawning, unbounded allocations, unbounded body reads"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // 1. Goroutines in for loops
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.for_query, tree.root_node(), source);
        let body_idx = find_capture_index(&self.for_query, "for_body");
        let stmt_idx = find_capture_index(&self.for_query, "for_stmt");

        while let Some(m) = matches.next() {
            let body = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let stmt = m
                .captures
                .iter()
                .find(|c| c.index as usize == stmt_idx)
                .map(|c| c.node);

            if let (Some(body), Some(stmt)) = (body, stmt) {
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut inner_matches =
                    inner_cursor.matches(&self.go_query, tree.root_node(), source);
                let go_stmt_idx = find_capture_index(&self.go_query, "go_stmt");

                while let Some(im) = inner_matches.next() {
                    let go_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == go_stmt_idx)
                        .map(|c| c.node);
                    if let Some(go_node) = go_node {
                        let start = go_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unbounded_goroutine_spawn".to_string(),
                            message: "goroutine spawned in loop without bound may exhaust resources"
                                .to_string(),
                            snippet: extract_snippet(source, stmt, 3),
                        });
                    }
                }
            }
        }

        // 2. ReadAll without MaxBytesReader
        let mut cursor2 = QueryCursor::new();
        let mut matches2 = cursor2.matches(&self.selector_query, tree.root_node(), source);
        let pkg_idx = find_capture_index(&self.selector_query, "pkg");
        let method_idx = find_capture_index(&self.selector_query, "method");
        let call_idx = find_capture_index(&self.selector_query, "call");

        while let Some(m) = matches2.next() {
            let pkg = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
            let method = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(pkg), Some(method), Some(call)) = (pkg, method, call) {
                let pkg_name = node_text(pkg, source);
                let method_name = node_text(method, source);

                if (pkg_name == "ioutil" || pkg_name == "io") && method_name == "ReadAll" {
                    let start = call.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unbounded_body_read".to_string(),
                        message:
                            "io.ReadAll without MaxBytesReader may exhaust memory on large input"
                                .to_string(),
                        snippet: extract_snippet(source, call, 1),
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
        let pipeline = GoResourceExhaustionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_goroutine_in_loop() {
        let src = r#"package main

func main() {
	for i := 0; i < n; i++ {
		go func() {}()
	}
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_goroutine_spawn");
    }

    #[test]
    fn detects_readall() {
        let src = r#"package main

func handler() {
	data, _ := io.ReadAll(r)
	_ = data
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_body_read");
    }

    #[test]
    fn detects_ioutil_readall() {
        let src = r#"package main

func handler() {
	data, _ := ioutil.ReadAll(r)
	_ = data
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_body_read");
    }

    #[test]
    fn clean_no_findings() {
        let src = r#"package main

func main() {
	go func() {
		doWork()
	}()
	fmt.Println("hello")
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
