use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::c_primitives::{compile_call_expression_query, extract_snippet, find_capture_index};

const UNSAFE_FUNCTIONS: &[&str] = &[
    "strcpy", "strcat", "sprintf", "vsprintf", "gets", "scanf",
];

pub struct BufferOverflowsPipeline {
    call_query: Arc<Query>,
}

impl BufferOverflowsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

impl Pipeline for BufferOverflowsPipeline {
    fn name(&self) -> &str {
        "buffer_overflows"
    }

    fn description(&self) -> &str {
        "Detects usage of unsafe string functions (strcpy, sprintf, gets, etc.) that can cause buffer overflows"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(fn_cap), Some(call_cap)) = (fn_cap, call_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                if !UNSAFE_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                let start = call_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "error".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unsafe_string_function".to_string(),
                    message: format!(
                        "`{fn_name}()` is unsafe — use bounded alternative (e.g. strncpy, snprintf)"
                    ),
                    snippet: extract_snippet(source, call_cap.node, 1),
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = BufferOverflowsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_strcpy() {
        let src = "void f() { strcpy(dest, src); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_string_function");
        assert!(findings[0].message.contains("strcpy"));
    }

    #[test]
    fn detects_sprintf() {
        let src = "void f() { sprintf(buf, \"%s\", name); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("sprintf"));
    }

    #[test]
    fn detects_gets() {
        let src = "void f() { gets(buf); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("gets"));
    }

    #[test]
    fn skips_safe_alternatives() {
        let src = "void f() { strncpy(dest, src, sizeof(dest)); snprintf(buf, sizeof(buf), \"%s\", name); memcpy(dest, src, n); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
