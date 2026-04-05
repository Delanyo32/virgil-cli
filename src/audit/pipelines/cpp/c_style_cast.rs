use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_cast_expression_query, extract_snippet, find_capture_index, node_text,
};

const NUMERIC_TYPES: &[&str] = &[
    "int", "float", "double", "char", "short", "long", "unsigned", "signed", "bool", "size_t",
    "uint8_t", "int8_t", "uint16_t", "int16_t", "uint32_t", "int32_t", "uint64_t", "int64_t",
    "ssize_t", "ptrdiff_t", "intptr_t", "uintptr_t",
];

pub struct CStyleCastPipeline {
    cast_query: Arc<Query>,
}

impl CStyleCastPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            cast_query: compile_cast_expression_query()?,
        })
    }

    fn classify_severity(cast_type: &str) -> Option<&'static str> {
        let trimmed = cast_type.trim();

        // (void)x is idiomatic for suppressing unused-var warnings — skip
        if trimmed == "void" {
            return None;
        }

        // Pointer casts are most dangerous
        if trimmed.contains('*') {
            return Some("error");
        }

        // Numeric casts are lower risk
        if NUMERIC_TYPES.contains(&trimmed) {
            return Some("info");
        }

        // Default
        Some("warning")
    }
}

impl NodePipeline for CStyleCastPipeline {
    fn name(&self) -> &str {
        "c_style_cast"
    }

    fn description(&self) -> &str {
        "Detects C-style casts — prefer static_cast/dynamic_cast/const_cast/reinterpret_cast"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.cast_query, tree.root_node(), source);

        let cast_expr_idx = find_capture_index(&self.cast_query, "cast_expr");
        let cast_type_idx = find_capture_index(&self.cast_query, "cast_type");

        while let Some(m) = matches.next() {
            let expr_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == cast_expr_idx);
            let type_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == cast_type_idx);

            if let (Some(expr_cap), Some(type_cap)) = (expr_cap, type_cap) {
                let cast_type = node_text(type_cap.node, source);

                let severity = match Self::classify_severity(cast_type) {
                    Some(s) => s,
                    None => continue, // void cast — skip
                };

                if is_nolint_suppressed(source, expr_cap.node, self.name()) {
                    continue;
                }

                let start = expr_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "c_style_cast".to_string(),
                    message: format!(
                        "C-style cast to `{cast_type}` — use `static_cast<>`, `dynamic_cast<>`, `const_cast<>`, or `reinterpret_cast<>`"
                    ),
                    snippet: extract_snippet(source, expr_cap.node, 1),
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CStyleCastPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_c_style_cast() {
        let src = "void f() { int x = (int)3.14; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "c_style_cast");
        assert!(findings[0].message.contains("int"));
    }

    #[test]
    fn no_finding_for_static_cast() {
        let src = "void f() { int x = static_cast<int>(3.14); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_pointer_cast() {
        let src = "void f(void* p) { int* ip = (int*)p; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn metadata_correct() {
        let src = "void f() { char c = (char)65; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
        assert_eq!(findings[0].pipeline, "c_style_cast");
    }

    #[test]
    fn void_cast_skipped() {
        let src = "void f(int x) { (void)x; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn pointer_cast_error() {
        let src = "void f(void* p) { int* ip = (int*)p; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn numeric_cast_info() {
        let src = "void f() { int x = (int)3.14; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn nolint_suppression() {
        let src = "void f() { int x = (int)3.14; // NOLINT }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
