use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{
    extract_snippet, find_capture_index, has_storage_class,
    is_cpp_forward_declaration, is_cpp_header, is_generated_cpp_file,
};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{is_entry_file, is_test_file};
use crate::language::Language;

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_ENTRY_FILES: &[&str] = &["main.cpp", "main.cc", "main.cxx"];

const CPP_DEFINITION_KINDS: &[&str] = &[
    "function_definition",
    "class_specifier",
    "struct_specifier",
    "enum_specifier",
    "union_specifier",
    "namespace_definition",
    "template_declaration",
    "type_definition",
    "declaration",
];

/// Count top-level C++ definitions, skipping forward declarations.
/// For `declaration` nodes: skip if `is_cpp_forward_declaration()` returns true.
/// For class/struct/enum/union specifiers: skip if they have no `body` field.
fn count_cpp_definitions_excluding_forward_decls(
    root: tree_sitter::Node,
    symbol_kinds: &[&str],
) -> usize {
    let mut count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if !symbol_kinds.contains(&child.kind()) {
            continue;
        }
        if child.kind() == "declaration" && is_cpp_forward_declaration(child) {
            continue;
        }
        if matches!(
            child.kind(),
            "class_specifier" | "struct_specifier" | "enum_specifier" | "union_specifier"
        ) && child.child_by_field_name("body").is_none()
        {
            continue;
        }
        count += 1;
    }
    count
}

fn cpp_lang() -> tree_sitter::Language {
    Language::Cpp.tree_sitter_language()
}

pub struct ModuleSizeDistributionPipeline {
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        // In C++, exported = non-static top-level definitions.
        // We query for top-level function_definition and declaration nodes,
        // then filter out those with `static` storage class at check time.
        let exported_query_str = r#"
[
  (function_definition) @def
  (declaration) @def
  (class_specifier name: (type_identifier)) @def
  (struct_specifier name: (type_identifier)) @def
  (enum_specifier name: (type_identifier)) @def
  (namespace_definition) @def
  (template_declaration) @def
  (type_definition) @def
]
"#;
        let exported_query = Query::new(&cpp_lang(), exported_query_str)
            .with_context(|| "failed to compile exported symbols query for C++ architecture")?;

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
        if is_test_file(file_path) {
            return vec![];
        }
        if is_generated_cpp_file(file_path, source) {
            return vec![];
        }

        let is_header = is_cpp_header(file_path);
        let mut findings = Vec::new();
        let root = tree.root_node();

        let total_definitions =
            count_cpp_definitions_excluding_forward_decls(root, CPP_DEFINITION_KINDS);
        let total_lines = source.split(|&b| b == b'\n').count();

        // Pattern 1: Oversized module
        // Headers aggregate declarations by design — apply 3x threshold.
        let oversized_threshold = if is_header {
            OVERSIZED_SYMBOL_THRESHOLD * 3 // 90
        } else {
            OVERSIZED_SYMBOL_THRESHOLD // 30
        };
        if total_definitions >= oversized_threshold || total_lines >= OVERSIZED_LINE_THRESHOLD {
            let severity = if total_definitions >= 120 {
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
                    total_definitions, total_lines, oversized_threshold, OVERSIZED_LINE_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: Monolithic export surface
        // Skip for header files — 100% export is expected and intentional.
        if !is_header {
            let mut exported_count = 0usize;
            let mut cursor = QueryCursor::new();
            let def_idx = find_capture_index(&self.exported_query, "def");
            let mut matches = cursor.matches(&self.exported_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == def_idx {
                        let is_top_level = cap
                            .node
                            .parent()
                            .is_some_and(|p| p.kind() == "translation_unit");
                        if is_top_level && !has_storage_class(cap.node, source, "static") {
                            exported_count += 1;
                        }
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
        if total_definitions == 1 && !is_entry_file(file_path, ANEMIC_ENTRY_FILES) {
            let snippet = {
                let mut cursor = root.walk();
                root.children(&mut cursor)
                    .find(|c| CPP_DEFINITION_KINDS.contains(&c.kind()))
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
                message:
                    "Module contains only 1 definition — consider merging into a related module"
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
        parser.set_language(&cpp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_oversized_module() {
        let mut src = String::new();
        for i in 0..31 {
            src.push_str(&format!("void func_{}() {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn no_oversized_for_small_module() {
        let src = "void foo() {}\nvoid bar() {}\nstruct Baz {};";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn detects_monolithic_export() {
        let mut src = String::new();
        for i in 0..21 {
            src.push_str(&format!("void func_{}() {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn no_monolithic_for_static_functions() {
        let mut src = String::new();
        for i in 0..21 {
            src.push_str(&format!("static void func_{}() {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn detects_anemic_module() {
        let src = "void getVersion() { return; }";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_entry_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&cpp_lang()).unwrap();
        let src = "int main() { return 0; }";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "main.cpp");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = "void foo() {}\nvoid bar() {}";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&cpp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn test_header_not_flagged_for_monolithic_export() {
        let mut src = String::new();
        for i in 0..25 {
            src.push_str(&format!("void func_{}(int x);\n", i));
        }
        let findings = parse_and_check_file(&src, "api.hpp");
        assert!(
            !findings.iter().any(|f| f.pattern == "monolithic_export_surface"),
            ".hpp should not trigger monolithic_export_surface"
        );
    }

    #[test]
    fn test_generated_protobuf_header_excluded() {
        let mut src = String::new();
        for i in 0..50 {
            src.push_str(&format!("void func_{}() {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "generated/user.pb.h");
        assert!(
            findings.is_empty(),
            "generated/user.pb.h should produce 0 findings, got: {:?}",
            findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_moc_generated_file_excluded() {
        let mut src = String::new();
        for i in 0..35 {
            src.push_str(&format!("void func_{}() {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "moc_mainwindow.cpp");
        assert!(findings.is_empty(), "moc_mainwindow.cpp should produce 0 findings");
    }

    #[test]
    fn test_forward_declaration_not_counted() {
        // class Foo; in tree-sitter-cpp is a class_specifier with no body
        // It should NOT count as a definition, so no anemic_module
        let src = "class Foo;";
        let findings = parse_and_check_file(src, "fwd.hpp");
        assert!(
            !findings.iter().any(|f| f.pattern == "anemic_module"),
            "A class forward declaration should not trigger anemic_module"
        );
    }

    #[test]
    fn test_severity_escalation_extreme_size() {
        // 120+ definitions → oversized_module with severity "error"
        let mut src = String::new();
        for i in 0..125 {
            src.push_str(&format!("void func_{}() {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "huge.cpp");
        let f = findings
            .iter()
            .find(|f| f.pattern == "oversized_module")
            .expect("125 definitions should trigger oversized_module");
        assert_eq!(f.severity, "error", "125 defs should be 'error' severity");
    }

    #[test]
    fn test_test_directory_excluded() {
        let src = "void test_connection_init() { /* verify */ }";
        let findings = parse_and_check_file(src, "tests/mock_socket.cpp");
        assert!(
            !findings.iter().any(|f| f.pattern == "anemic_module"),
            "Files in tests/ should be excluded from anemic_module"
        );
    }

    #[test]
    fn test_header_oversized_higher_threshold() {
        // 31 definitions — exceeds standard threshold (30) but below header threshold (90)
        let mut src = String::new();
        for i in 0..31 {
            src.push_str(&format!("void func_{}(int x);\n", i));
        }
        let findings = parse_and_check_file(&src, "api.hpp");
        assert!(
            !findings.iter().any(|f| f.pattern == "oversized_module"),
            ".hpp with 31 defs should not fire (header threshold = 90)"
        );
    }
}
