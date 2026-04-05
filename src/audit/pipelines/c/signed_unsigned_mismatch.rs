use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_for_statement_query, extract_snippet, find_capture_index, node_text,
};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

/// Functions known to return size_t or unsigned values.
const SIZE_FUNCTIONS: &[&str] = &[
    "strlen", "sizeof", "wcslen", "strnlen", "fread", "fwrite", "offsetof",
];

/// Signed integer type specifiers (the counter type must be one of these).
const SIGNED_TYPES: &[&str] = &[
    "int", "short", "long", "char", "signed",
];

pub struct SignedUnsignedMismatchPipeline {
    for_query: Arc<Query>,
}

impl SignedUnsignedMismatchPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            for_query: compile_for_statement_query()?,
        })
    }

    /// Check if the for-loop init declares a signed integer counter.
    fn init_type_is_signed(init_node: tree_sitter::Node, source: &[u8]) -> bool {
        if let Some(type_node) = init_node.child_by_field_name("type") {
            let type_text = node_text(type_node, source).trim();
            // Skip unsigned types
            if type_text.contains("unsigned") || type_text == "size_t" {
                return false;
            }
            // Check if it's a known signed type
            for signed_type in SIGNED_TYPES {
                if type_text == *signed_type || type_text.contains(signed_type) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if the condition's RHS is a genuinely unsigned-returning expression.
    /// Walk the condition AST looking for sizeof expressions or calls to size-returning functions.
    fn condition_has_unsigned_rhs(cond_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk all nodes in the condition subtree
        Self::subtree_has_unsigned(cond_node, source)
    }

    fn subtree_has_unsigned(node: tree_sitter::Node, source: &[u8]) -> bool {
        // sizeof expression always returns size_t
        if node.kind() == "sizeof_expression" {
            return true;
        }

        // Call to a known size-returning function
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
            && SIZE_FUNCTIONS.contains(&node_text(func, source))
        {
            return true;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::subtree_has_unsigned(child, source) {
                return true;
            }
        }
        false
    }
}

impl GraphPipeline for SignedUnsignedMismatchPipeline {
    fn name(&self) -> &str {
        "signed_unsigned_mismatch"
    }

    fn description(&self) -> &str {
        "Detects for-loops using signed int counters compared against unsigned size values"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
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
                // Check: init declares a signed integer counter
                if !Self::init_type_is_signed(init_cap.node, source) {
                    continue;
                }

                // Check: condition RHS involves a genuinely unsigned expression
                if !Self::condition_has_unsigned_rhs(cond_cap.node, source) {
                    continue;
                }

                if is_nolint_suppressed(source, stmt_cap.node, self.name()) {
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
                    message: "signed loop counter compared with unsigned size — use `size_t` to avoid sign mismatch".to_string(),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SignedUnsignedMismatchPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.c",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_int_vs_strlen() {
        let src = "void f(const char *s) { for (int i = 0; i < strlen(s); i++) {} }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "signed_unsigned_mismatch");
    }

    #[test]
    fn no_false_positive_on_substring() {
        // "recount" contains "count" as substring but is just a signed int variable
        let src = "void f(int recount) { for (int i = 0; i < recount; i++) {} }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
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

    #[test]
    fn detects_long_counter_vs_strlen() {
        let src = "void f(const char *s) { for (long i = 0; i < strlen(s); i++) {} }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_unsigned_counter() {
        let src = "void f(const char *s) { for (unsigned int i = 0; i < strlen(s); i++) {} }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_int_vs_sizeof() {
        let src = "void f() { for (int i = 0; i < sizeof(arr); i++) {} }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn nolint_suppresses() {
        let src = "void f(const char *s) { for (int i = 0; i < strlen(s); i++) {} } // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
