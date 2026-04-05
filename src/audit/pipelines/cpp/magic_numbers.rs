use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use crate::audit::pipelines::helpers::COMMON_ALLOWED_NUMBERS;

use super::primitives::{
    compile_numeric_literal_query, find_capture_index, has_constexpr, has_type_qualifier,
};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", "-1", "10", "100", "1000", "256", "512", "1024", "2048", "4096",
    "8192", "0xFF", "0xff", "0x80", "0xFFFF", "0xffff",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &[
    "preproc_def",
    "preproc_function_def",
    "enumerator",
    "template_argument_list",
    "bitfield_clause",
    "field_declaration",
    "array_declarator",
    "initializer_list",
    "static_assert",
    "case_statement",
    "condition_clause",
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

    fn strip_numeric_suffix(value: &str) -> &str {
        let v = value.trim();
        // Handle float suffixes (only if value looks like a float)
        if v.contains('.') || v.contains('e') || v.contains('E') {
            for suffix in &["f", "F", "l", "L"] {
                if let Some(stripped) = v.strip_suffix(suffix) {
                    return stripped;
                }
            }
            return v;
        }
        // Handle integer suffixes (check longer ones first)
        for suffix in &[
            "ull", "ULL", "Ull", "uLL", "ul", "UL", "Ul", "uL", "ll", "LL", "u", "U", "l", "L",
        ] {
            if let Some(stripped) = v.strip_suffix(suffix) {
                return stripped;
            }
        }
        v
    }

    fn is_exempt_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            let kind = parent.kind();

            if EXEMPT_ANCESTOR_KINDS.contains(&kind) {
                return true;
            }

            // Exempt: declaration with const qualifier or constexpr
            if kind == "declaration"
                && (has_type_qualifier(parent, source, "const") || has_constexpr(parent, source))
            {
                return true;
            }

            current = parent.parent();
        }

        // Skip if inside subscript_argument_list (array indexing)
        if let Some(parent) = node.parent()
            && parent.kind() == "subscript_argument_list"
        {
            return true;
        }

        false
    }

    fn is_test_file(file_path: &str) -> bool {
        let lower = file_path.to_lowercase();
        lower.contains("_test.") || lower.contains("test_") || lower.contains("/tests/") || lower.contains("/spec/")
    }
}

impl NodePipeline for CppMagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals outside const/constexpr/enum/macro contexts that should be named constants"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if Self::is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.numeric_query, tree.root_node(), source);

        let number_idx = find_capture_index(&self.numeric_query, "number");

        const MAX_FINDINGS_PER_FILE: usize = 200;

        while let Some(m) = matches.next() {
            if findings.len() >= MAX_FINDINGS_PER_FILE {
                break;
            }
            let num_cap = m.captures.iter().find(|c| c.index as usize == number_idx);

            if let Some(num_cap) = num_cap {
                let value = num_cap.node.utf8_text(source).unwrap_or("");
                let normalized = Self::strip_numeric_suffix(value);

                if EXCLUDED_VALUES.contains(&normalized) || COMMON_ALLOWED_NUMBERS.contains(&normalized) {
                    continue;
                }

                // Also check case-insensitive hex
                let lower = normalized.to_lowercase();
                if EXCLUDED_VALUES.iter().any(|v| v.to_lowercase() == lower) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node, source) {
                    continue;
                }

                if is_nolint_suppressed(source, num_cap.node, self.name()) {
                    continue;
                }

                // Graduate severity: array sizes get "warning"
                let severity = if let Some(parent) = num_cap.node.parent()
                    && parent.kind() == "array_declarator"
                {
                    "warning"
                } else {
                    "info"
                };

                let start = num_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
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
        parse_and_check_file(source, "test.cpp")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CppMagicNumbersPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
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

    #[test]
    fn number_with_suffix_excluded() {
        let src = "void f() { unsigned x = 100U; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn switch_case_exempt() {
        let src = "void f(int x) { switch(x) { case 42: break; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression() {
        let src = "void f() { int x = 42; // NOLINT }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_file_skipped() {
        let src = "void f() { int x = 42; }";
        let findings = parse_and_check_file(src, "test_math.cpp");
        assert!(findings.is_empty());
    }

    #[test]
    fn hex_case_insensitive() {
        let src = "void f() { int x = 0XFF; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
