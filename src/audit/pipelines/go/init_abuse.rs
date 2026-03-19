use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_decl_query, compile_selector_call_query, extract_snippet, find_capture_index,
    node_text,
};

const SUSPICIOUS_CALLS: &[(&str, &str)] = &[
    ("sql", "Open"),
    ("http", "Get"),
    ("http", "Post"),
    ("http", "ListenAndServe"),
    ("os", "Open"),
    ("os", "Create"),
    ("log", "Fatal"),
    ("log", "Fatalf"),
    ("net", "Listen"),
    ("net", "Dial"),
];

pub struct InitAbusePipeline {
    fn_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl InitAbusePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_decl_query()?,
            call_query: compile_selector_call_query()?,
        })
    }
}

impl Pipeline for InitAbusePipeline {
    fn name(&self) -> &str {
        "init_abuse"
    }

    fn description(&self) -> &str {
        "Detects side effects (DB, HTTP, file I/O, log.Fatal) in init() functions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // First, find all init() function bodies
        let mut fn_cursor = QueryCursor::new();
        let mut fn_matches = fn_cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");
        let fn_body_idx = find_capture_index(&self.fn_query, "fn_body");

        let mut init_body_ranges: Vec<tree_sitter::Range> = Vec::new();

        while let Some(m) = fn_matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_body_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node)) = (name_node, body_node) {
                if node_text(name_node, source) == "init" {
                    init_body_ranges.push(body_node.range());
                }
            }
        }

        if init_body_ranges.is_empty() {
            return findings;
        }

        // Now find suspicious calls and check if they're inside init()
        let mut call_cursor = QueryCursor::new();
        let mut call_matches = call_cursor.matches(&self.call_query, tree.root_node(), source);

        let pkg_idx = find_capture_index(&self.call_query, "pkg");
        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = call_matches.next() {
            let pkg_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
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

            if let (Some(pkg_node), Some(method_node), Some(call_node)) =
                (pkg_node, method_node, call_node)
            {
                let pkg_name = node_text(pkg_node, source);
                let method_name = node_text(method_node, source);

                let is_suspicious = SUSPICIOUS_CALLS
                    .iter()
                    .any(|(p, m)| *p == pkg_name && *m == method_name);

                if !is_suspicious {
                    continue;
                }

                // Check if call is inside an init() body
                let call_start = call_node.start_byte();
                let in_init = init_body_ranges
                    .iter()
                    .any(|range| call_start >= range.start_byte && call_start <= range.end_byte);

                if !in_init {
                    continue;
                }

                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "init_side_effect".to_string(),
                    message: format!(
                        "side-effect call `{pkg_name}.{method_name}()` in init() — move to explicit initialization"
                    ),
                    snippet: extract_snippet(source, call_node, 1),
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
        let pipeline = InitAbusePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_sql_open_in_init() {
        let src = "package main\nfunc init() {\n\tdb, _ := sql.Open(\"postgres\", \"dsn\")\n\t_ = db\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "init_side_effect");
    }

    #[test]
    fn clean_init_with_variable_only() {
        let src =
            "package main\nvar defaultTimeout = 30\nfunc init() {\n\tdefaultTimeout = 60\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_regular_function() {
        let src = "package main\nfunc setup() {\n\tdb, _ := sql.Open(\"postgres\", \"dsn\")\n\t_ = db\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_log_fatal_in_init() {
        let src = "package main\nfunc init() {\n\tlog.Fatal(\"startup failed\")\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }
}
