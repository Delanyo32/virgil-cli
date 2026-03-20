use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_for_statement_query, extract_snippet, find_capture_index, node_text,
};

const SIZE_LIKE_IDENTIFIERS: &[&str] = &["size", "len", "length", "count", "num", "sz"];
const SIZE_FUNCTIONS: &[&str] = &["strlen", "sizeof", "wcslen"];

pub struct SignedUnsignedMismatchPipeline {
    for_query: Arc<Query>,
}

impl SignedUnsignedMismatchPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            for_query: compile_for_statement_query()?,
        })
    }

    fn init_type_is_int(init_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Check if the declaration type is "int" (signed)
        if let Some(type_node) = init_node.child_by_field_name("type") {
            let type_text = node_text(type_node, source).trim();
            return type_text == "int";
        }
        false
    }

    fn condition_compares_size(cond_node: tree_sitter::Node, source: &[u8]) -> bool {
        let cond_text = node_text(cond_node, source);

        // Check for size-like identifiers
        for ident in SIZE_LIKE_IDENTIFIERS {
            if cond_text.contains(ident) {
                return true;
            }
        }

        // Check for size function calls
        for func in SIZE_FUNCTIONS {
            if cond_text.contains(func) {
                return true;
            }
        }

        false
    }
}

impl Pipeline for SignedUnsignedMismatchPipeline {
    fn name(&self) -> &str {
        "signed_unsigned_mismatch"
    }

    fn description(&self) -> &str {
        "Detects for-loops using signed int counters compared against unsigned size values"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.for_query, tree.root_node(), source);

        let for_init_idx = find_capture_index(&self.for_query, "for_init");
        let for_cond_idx = find_capture_index(&self.for_query, "for_cond");
        let for_stmt_idx = find_capture_index(&self.for_query, "for_stmt");

        while let Some(m) = matches.next() {
            let init_cap = m.captures.iter().find(|c| c.index as usize == for_init_idx);
            let cond_cap = m.captures.iter().find(|c| c.index as usize == for_cond_idx);
            let stmt_cap = m.captures.iter().find(|c| c.index as usize == for_stmt_idx);

            if let (Some(init_cap), Some(cond_cap), Some(stmt_cap)) = (init_cap, cond_cap, stmt_cap)
            {
                // Check: init declares `int` variable
                if !Self::init_type_is_int(init_cap.node, source) {
                    continue;
                }

                // Check: condition compares against size-like expression
                if !Self::condition_compares_size(cond_cap.node, source) {
                    continue;
                }

                let start = stmt_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "signed_unsigned_mismatch".to_string(),
                    message: "signed `int` loop counter compared with unsigned size — use `size_t` to avoid sign mismatch".to_string(),
                    snippet: extract_snippet(source, stmt_cap.node, 1),
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
        let pipeline = SignedUnsignedMismatchPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_int_vs_strlen() {
        let src = "void f(const char *s) { for (int i = 0; i < strlen(s); i++) {} }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "signed_unsigned_mismatch");
    }

    #[test]
    fn detects_int_vs_size() {
        let src = "void f(int size) { for (int i = 0; i < size; i++) {} }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_size_t_counter() {
        let src = "void f(int len) { for (size_t i = 0; i < len; i++) {} }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_int_vs_literal() {
        let src = "void f() { for (int i = 0; i < 10; i++) {} }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
