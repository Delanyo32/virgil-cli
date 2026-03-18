use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_numeric_literal_query, find_capture_index, has_constexpr, has_type_qualifier,
};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", "-1",
    "10", "100", "1000",
    "256", "512", "1024", "2048", "4096", "8192",
    "0xFF", "0xff", "0x80", "0xFFFF", "0xffff",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &[
    "preproc_def",
    "preproc_function_def",
    "enumerator",
    "template_argument_list",
    "bitfield_clause",
    "field_declaration",
    "array_declarator",
];

pub struct CppMagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl CppMagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: compile_numeric_literal_query()?,
        })
    }

    fn is_exempt_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            let kind = parent.kind();

            if EXEMPT_ANCESTOR_KINDS.contains(&kind) {
                return true;
            }

            // Exempt: declaration with const qualifier or constexpr
            if kind == "declaration" {
                if has_type_qualifier(parent, source, "const") || has_constexpr(parent, source) {
                    return true;
                }
            }

            current = parent.parent();
        }

        // Skip if inside subscript_argument_list (array indexing)
        if let Some(parent) = node.parent() {
            if parent.kind() == "subscript_argument_list" {
                return true;
            }
        }

        false
    }
}

impl Pipeline for CppMagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals outside const/constexpr/enum/macro contexts that should be named constants"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.numeric_query, tree.root_node(), source);

        let number_idx = find_capture_index(&self.numeric_query, "number");

        while let Some(m) = matches.next() {
            let num_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == number_idx);

            if let Some(num_cap) = num_cap {
                let value = num_cap.node.utf8_text(source).unwrap_or("");

                if EXCLUDED_VALUES.contains(&value) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node, source) {
                    continue;
                }

                let start = num_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "magic_number".to_string(),
                    message: format!(
                        "magic number `{value}` — consider extracting to a named constant or constexpr"
                    ),
                    snippet: value.to_string(),
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
        let pipeline = CppMagicNumbersPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_magic_number() {
        let src = "void f() { int x = 42; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_number");
        assert!(findings[0].message.contains("42"));
    }

    #[test]
    fn skips_const() {
        let src = "const int MAX = 100;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_constexpr() {
        let src = "constexpr int SIZE = 256;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_enum() {
        let src = "enum { FOO = 42, BAR = 99 };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_define() {
        let src = "#define MAX_SIZE 100";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let src = "void f() { int x = 0; int y = 1; int z = 2; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_array_index() {
        let src = "void f() { int x = arr[3]; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_template_argument() {
        let src = "std::array<int, 10> arr;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
