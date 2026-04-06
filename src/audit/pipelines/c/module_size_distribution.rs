use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{
    extract_snippet, find_capture_index, has_storage_class, is_c_forward_declaration,
    is_generated_c_file,
};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{is_entry_file, is_test_file};
use crate::language::Language;

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_THRESHOLD: usize = 1;
const ANEMIC_ENTRY_FILES: &[&str] = &["main.c"];

const C_DEFINITION_KINDS: &[&str] = &[
    "function_definition",
    "struct_specifier",
    "enum_specifier",
    "union_specifier",
    "type_definition",
    "declaration",
];

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

/// Count top-level definitions, excluding forward declarations.
/// For `declaration` nodes: forward declarations have no compound_statement or init_declarator.
/// For struct/enum/union specifiers: forward declarations have no body field.
fn count_top_level_definitions_excluding_forward_decls(
    root: tree_sitter::Node,
    symbol_kinds: &[&str],
) -> usize {
    let mut count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if !symbol_kinds.contains(&child.kind()) {
            continue;
        }
        // Skip declaration nodes that are forward declarations
        if child.kind() == "declaration" && is_c_forward_declaration(child) {
            continue;
        }
        // Skip struct/enum/union specifiers without a body (forward declarations)
        if (child.kind() == "struct_specifier"
            || child.kind() == "enum_specifier"
            || child.kind() == "union_specifier")
            && child.child_by_field_name("body").is_none()
        {
            continue;
        }
        count += 1;
    }
    count
}

pub struct ModuleSizeDistributionPipeline {
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        let exported_query_str = r#"
[
  (function_definition) @def
  (declaration) @def
  (struct_specifier) @def
  (enum_specifier) @def
  (union_specifier) @def
  (type_definition) @def
]
"#;
        let exported_query = Query::new(&c_lang(), exported_query_str)
            .with_context(|| "failed to compile exported symbols query for C architecture")?;

        Ok(Self {
            exported_query: Arc::new(exported_query),
        })
    }
}

impl Pipeline for ModuleSizeDistributionPipeline {
    fn name(&self) -> &str {
        "module_size_distribution"
    }

    fn description(&self) -> &str {
        "Detects oversized modules, monolithic export surfaces, and anemic modules"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        // Skip test files — single-definition test helpers are expected
        if is_test_file(file_path) {
            return vec![];
        }
        // Skip generated files — they are not candidates for refactoring
        if is_generated_c_file(file_path, source) {
            return vec![];
        }

        let is_header = file_path.ends_with(".h");
        let mut findings = Vec::new();
        let root = tree.root_node();

        let total_definitions =
            count_top_level_definitions_excluding_forward_decls(root, C_DEFINITION_KINDS);
        let total_lines = source.split(|&b| b == b'\n').count();

        // Pattern 1: Oversized module (skip for header files — large headers are by design)
        if !is_header
            && (total_definitions >= OVERSIZED_SYMBOL_THRESHOLD
                || total_lines >= OVERSIZED_LINE_THRESHOLD)
        {
            let severity = if total_definitions >= 151 {
                "error"
            } else if total_definitions >= 61 {
                "warning"
            } else {
                "info"
            };
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: severity.to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "oversized_module".to_string(),
                message: format!(
                    "Module has {} definitions and {} lines (thresholds: {} definitions or {} lines)",
                    total_definitions,
                    total_lines,
                    OVERSIZED_SYMBOL_THRESHOLD,
                    OVERSIZED_LINE_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: Monolithic export surface (skip for header files — headers ARE the export surface)
        if !is_header {
            let mut exported_count = 0usize;
            let mut cursor = QueryCursor::new();
            let def_idx = find_capture_index(&self.exported_query, "def");
            let mut matches = cursor.matches(&self.exported_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == def_idx
                        && cap
                            .node
                            .parent()
                            .is_some_and(|p| p.kind() == "translation_unit")
                        && !has_storage_class(cap.node, source, "static")
                    {
                        exported_count += 1;
                    }
                }
            }

            if exported_count >= MONOLITHIC_EXPORT_THRESHOLD {
                let severity = if exported_count >= 41 { "warning" } else { "info" };
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: 1,
                    column: 1,
                    severity: severity.to_string(),
                    pipeline: "module_size_distribution".to_string(),
                    pattern: "monolithic_export_surface".to_string(),
                    message: format!(
                        "Module exports {} symbols (threshold: {})",
                        exported_count, MONOLITHIC_EXPORT_THRESHOLD
                    ),
                    snippet: String::new(),
                });
            }
        }

        // Pattern 3: Anemic module (test files already excluded by early return above)
        if total_definitions == ANEMIC_THRESHOLD && !is_entry_file(file_path, ANEMIC_ENTRY_FILES) {
            let snippet = {
                let mut cursor = root.walk();
                root.children(&mut cursor)
                    .find(|c| C_DEFINITION_KINDS.contains(&c.kind()))
                    .map(|n| extract_snippet(source, n, 3))
                    .unwrap_or_default()
            };
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "anemic_module".to_string(),
                message: "Module contains only 1 definition — consider merging into a related module"
                    .to_string(),
                snippet,
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_oversized_module() {
        let mut src = String::new();
        for i in 0..31 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn no_oversized_for_small_module() {
        let src = "void foo(void) {}\nvoid bar(void) {}\n";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn detects_monolithic_export() {
        let mut src = String::new();
        for i in 0..21 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "monolithic_export_surface"));
    }

    #[test]
    fn no_monolithic_for_static_functions() {
        let mut src = String::new();
        for i in 0..21 {
            src.push_str(&format!("static void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(!findings.iter().any(|f| f.pattern == "monolithic_export_surface"));
    }

    #[test]
    fn detects_anemic_module() {
        let src = "int get_max_retries(void) { return 5; }";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_entry_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
        let src = "int main(void) { return 0; }";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "main.c");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = "void foo(void) {}\nvoid bar(void) {}\n";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    // ── New tests ──────────────────────────────────────────────────────────

    #[test]
    fn test_header_file_skips_oversized() {
        let mut src = String::new();
        for i in 0..35 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "large_api.h");
        assert!(
            !findings.iter().any(|f| f.pattern == "oversized_module"),
            "Header files should skip oversized_module"
        );
        assert!(
            !findings.iter().any(|f| f.pattern == "monolithic_export_surface"),
            "Header files should skip monolithic_export_surface"
        );
    }

    #[test]
    fn test_generated_file_suppressed() {
        let mut src = String::new();
        for i in 0..60 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "lex.yy.c");
        assert!(findings.is_empty(), "lex.yy.c should produce 0 findings");
    }

    #[test]
    fn test_tab_c_generated_file_suppressed() {
        let mut src = String::new();
        for i in 0..50 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "parser.tab.c");
        assert!(findings.is_empty(), "parser.tab.c should produce 0 findings");
    }

    #[test]
    fn test_macros_not_counted_in_definitions() {
        let mut src = String::new();
        for i in 0..35 {
            src.push_str(&format!("#define MACRO_{} {}\n", i, i));
        }
        src.push_str("void a(void) {}\nvoid b(void) {}\nvoid c(void) {}\n");
        let findings = parse_and_check_file(&src, "macros.c");
        assert!(
            !findings.iter().any(|f| f.pattern == "oversized_module"),
            "Macro definitions should not count toward oversized_module"
        );
    }

    #[test]
    fn test_oversized_by_line_count_only() {
        let mut src = String::new();
        for _ in 0..996 {
            src.push_str("// comment\n");
        }
        for i in 0..5 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "comments.c");
        assert!(
            findings.iter().any(|f| f.pattern == "oversized_module"),
            "Line count >= 1000 should trigger oversized_module even with few definitions"
        );
    }

    #[test]
    fn test_oversized_severity_graduation() {
        let mut src_small = String::new();
        for i in 0..31 {
            src_small.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings_small = parse_and_check_file(&src_small, "borderline.c");
        let f_small = findings_small
            .iter()
            .find(|f| f.pattern == "oversized_module")
            .expect("31 definitions should trigger oversized_module");
        assert_eq!(f_small.severity, "info", "31 definitions → 'info' severity");

        let mut src_large = String::new();
        for i in 0..200 {
            src_large.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings_large = parse_and_check_file(&src_large, "massive.c");
        let f_large = findings_large
            .iter()
            .find(|f| f.pattern == "oversized_module")
            .expect("200 definitions should trigger oversized_module");
        assert_eq!(f_large.severity, "error", "200 definitions → 'error' severity");
    }

    #[test]
    fn test_anemic_non_main_entry_file() {
        let src = "int get_max_retries(void) { return 5; }";
        let findings = parse_and_check_file(src, "program.c");
        assert!(
            findings.iter().any(|f| f.pattern == "anemic_module"),
            "program.c with 1 definition should trigger anemic_module"
        );
    }

    #[test]
    fn test_no_anemic_in_test_directory() {
        let src = "void test_connection_setup(void) { /* verify */ }";
        let findings = parse_and_check_file(src, "tests/mock_handler.c");
        assert!(
            !findings.iter().any(|f| f.pattern == "anemic_module"),
            "Test directory files should be excluded from anemic_module"
        );
    }

    #[test]
    fn test_forward_declarations_excluded_from_count() {
        let src = r#"
struct Foo;
struct Bar;
struct Baz;
struct Qux;
struct Quux;
void func_a(void) {}
void func_b(void) {}
"#;
        // 5 struct forward declarations (no body) + 2 definitions = 2 total < 30
        let findings = parse_and_check_file(src, "module.c");
        assert!(
            !findings.iter().any(|f| f.pattern == "oversized_module"),
            "Struct forward declarations should not count toward the definition threshold"
        );
    }

    #[test]
    fn test_both_patterns_fire_on_same_large_exported_file() {
        let mut src = String::new();
        for i in 0..35 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "big_api.c");
        assert!(
            findings.iter().any(|f| f.pattern == "oversized_module"),
            "35 functions should trigger oversized_module"
        );
        assert!(
            findings.iter().any(|f| f.pattern == "monolithic_export_surface"),
            "35 non-static functions should trigger monolithic_export_surface"
        );
    }
}
