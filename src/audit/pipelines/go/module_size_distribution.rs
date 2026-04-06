use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    count_top_level_definitions, is_entry_file, is_generated_go_file, is_nolint_suppressed,
    is_test_file,
};
use crate::language::Language;

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_ENTRY_FILES: &[&str] = &["main.go", "doc.go"];

const GO_DEFINITION_KINDS: &[&str] = &[
    "function_declaration",
    "method_declaration",
    "type_declaration",
    "var_declaration",
    "const_declaration",
];

fn go_lang() -> tree_sitter::Language {
    Language::Go.tree_sitter_language()
}

/// Like `count_top_level_definitions` but expands `const_declaration` and `var_declaration`
/// groups by counting their individual `const_spec`/`var_spec` children.
/// This prevents false-positive `anemic_module` findings for files like:
///   const ( A=1; B=2; C=3 )   <- one const_declaration node, three specs
fn count_expanded_top_level_defs(root: tree_sitter::Node) -> usize {
    let mut count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "method_declaration" | "type_declaration" => {
                count += 1;
            }
            "const_declaration" => {
                // Grammar may wrap specs in const_spec_list or put them directly
                let mut c = child.walk();
                let mut specs = 0usize;
                for sub in child.children(&mut c) {
                    if sub.kind() == "const_spec" {
                        specs += 1;
                    } else if sub.kind() == "const_spec_list" {
                        let mut c2 = sub.walk();
                        specs += sub.children(&mut c2).filter(|n| n.kind() == "const_spec").count();
                    }
                }
                count += specs.max(1);
            }
            "var_declaration" => {
                // Grammar wraps specs in var_spec_list or puts them directly
                let mut c = child.walk();
                let mut specs = 0usize;
                for sub in child.children(&mut c) {
                    if sub.kind() == "var_spec" {
                        specs += 1;
                    } else if sub.kind() == "var_spec_list" {
                        let mut c2 = sub.walk();
                        specs += sub.children(&mut c2).filter(|n| n.kind() == "var_spec").count();
                    }
                }
                count += specs.max(1);
            }
            _ => {}
        }
    }
    count
}

pub struct ModuleSizeDistributionPipeline {
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        // Query to find named declarations whose names we can inspect for uppercase
        let exported_query_str = r#"
[
  (function_declaration name: (identifier) @name) @def
  (method_declaration name: (field_identifier) @name) @def
  (type_declaration (type_spec name: (type_identifier) @name)) @def
  (var_declaration (var_spec name: (identifier) @name)) @def
  (const_declaration (const_spec name: (identifier) @name)) @def
]
"#;
        let exported_query = Query::new(&go_lang(), exported_query_str)
            .with_context(|| "failed to compile exported symbols query for Go architecture")?;

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
        // Skip test files
        if is_test_file(file_path) {
            return Vec::new();
        }

        // Skip generated files (.pb.go, _gen.go, "// Code generated" header, etc.)
        if is_generated_go_file(file_path, source) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let root = tree.root_node();

        // Check for NOLINT suppression on the package declaration line
        {
            let mut cursor = root.walk();
            let suppressed = root
                .children(&mut cursor)
                .find(|c| c.kind() == "package_clause")
                .is_some_and(|pkg| is_nolint_suppressed(source, pkg, self.name()));
            if suppressed {
                return Vec::new();
            }
        }

        let total_definitions = count_top_level_definitions(root, GO_DEFINITION_KINDS);
        let total_lines = source.split(|&b| b == b'\n').count();

        // Pattern 1: Oversized module — severity graduated by definition count
        if total_definitions >= OVERSIZED_SYMBOL_THRESHOLD
            || total_lines >= OVERSIZED_LINE_THRESHOLD
        {
            let severity = if total_definitions >= 100 {
                "error"
            } else if total_definitions >= 50 || total_lines >= OVERSIZED_LINE_THRESHOLD {
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
                    total_definitions, total_lines, OVERSIZED_SYMBOL_THRESHOLD, OVERSIZED_LINE_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: Monolithic export surface
        // In Go, exported symbols start with an uppercase letter
        let mut exported_count = 0usize;
        let mut cursor = QueryCursor::new();
        let name_idx = find_capture_index(&self.exported_query, "name");
        let def_idx = find_capture_index(&self.exported_query, "def");
        let mut matches = cursor.matches(&self.exported_query, root, source);
        while let Some(m) = matches.next() {
            let mut is_top_level = false;
            let mut is_exported = false;

            for cap in m.captures {
                if cap.index as usize == def_idx {
                    // Only count top-level definitions
                    if cap.node.parent().is_some_and(|p| p.kind() == "source_file") {
                        is_top_level = true;
                    }
                }
                if cap.index as usize == name_idx {
                    let text = node_text(cap.node, source);
                    if text.starts_with(|c: char| c.is_ascii_uppercase()) {
                        is_exported = true;
                    }
                }
            }

            if is_top_level && is_exported {
                exported_count += 1;
            }
        }

        if exported_count >= MONOLITHIC_EXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "module_size_distribution".to_string(),
                pattern: "monolithic_export_surface".to_string(),
                message: format!(
                    "Module exports {} symbols (threshold: {})",
                    exported_count, MONOLITHIC_EXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 3: Anemic module
        // Use expanded count so const/var blocks with multiple identifiers are not anemic
        let expanded_defs = count_expanded_top_level_defs(root);
        let is_test_file_path = file_path.ends_with("_test.go");
        if expanded_defs == 1 && !is_entry_file(file_path, ANEMIC_ENTRY_FILES) && !is_test_file_path
        {
            let snippet = {
                let mut cursor = root.walk();
                root.children(&mut cursor)
                    .find(|c| GO_DEFINITION_KINDS.contains(&c.kind()))
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
        parser.set_language(&go_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_oversized_module() {
        let mut src = String::from("package main\n");
        for i in 0..31 {
            src.push_str(&format!("func func_{}() {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn no_oversized_for_small_module() {
        let src = "package main\nfunc foo() {}\nfunc bar() {}\ntype Baz struct{}";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn detects_monolithic_export() {
        let mut src = String::from("package main\n");
        for i in 0..21 {
            src.push_str(&format!("func Func{}() {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn no_monolithic_for_unexported() {
        let mut src = String::from("package main\n");
        for i in 0..25 {
            src.push_str(&format!("func func{}() {{}}\n", i));
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
        let src = "package app\nconst Version = \"1.0.0\"";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_entry_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let src = "package main\nfunc main() {}";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "main.go");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_test_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let src = "package app\nfunc TestSomething() {}";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "app_test.go");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = "package app\nfunc foo() {}\nfunc bar() {}";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn test_doc_go_not_anemic() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let src = "// Package mypackage provides utilities.\npackage mypackage\n";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "doc.go");
        assert!(
            !findings.iter().any(|f| f.pattern == "anemic_module"),
            "doc.go should not be flagged as anemic_module"
        );
    }

    #[test]
    fn test_generated_file_skipped() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let src = "// Code generated by protoc-gen-go. DO NOT EDIT.\npackage main\n".to_string()
            + &(0..40).map(|i| format!("func Func{}() {{}}\n", i)).collect::<String>();
        let tree = parser.parse(&src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "service.go");
        assert!(findings.is_empty(), "generated file should produce no findings, got: {:?}", findings);
    }

    #[test]
    fn test_pb_go_file_skipped() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let src = "package proto\n".to_string()
            + &(0..35).map(|i| format!("func Func{}() {{}}\n", i)).collect::<String>();
        let tree = parser.parse(&src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "proto/user.pb.go");
        assert!(findings.is_empty(), ".pb.go file should produce no findings, got: {:?}", findings);
    }

    #[test]
    fn test_oversized_at_threshold_is_info() {
        let mut src = String::from("package main\n");
        for i in 0..30 { src.push_str(&format!("func func_{}() {{}}\n", i)); }
        let findings = parse_and_check(&src);
        let f = findings.iter().find(|f| f.pattern == "oversized_module");
        assert!(f.is_some(), "30 definitions should trigger oversized_module");
        assert_eq!(f.unwrap().severity, "info", "30-49 definitions should be 'info'");
    }

    #[test]
    fn test_oversized_50_defs_is_warning() {
        let mut src = String::from("package main\n");
        for i in 0..50 { src.push_str(&format!("func func_{}() {{}}\n", i)); }
        let findings = parse_and_check(&src);
        let f = findings.iter().find(|f| f.pattern == "oversized_module");
        assert!(f.is_some());
        assert_eq!(f.unwrap().severity, "warning", "50-99 definitions should be 'warning'");
    }

    #[test]
    fn test_oversized_100_defs_is_error() {
        let mut src = String::from("package main\n");
        for i in 0..100 { src.push_str(&format!("func func_{}() {{}}\n", i)); }
        let findings = parse_and_check(&src);
        let f = findings.iter().find(|f| f.pattern == "oversized_module");
        assert!(f.is_some());
        assert_eq!(f.unwrap().severity, "error", "100+ definitions should be 'error'");
    }

    #[test]
    fn test_nolint_suppresses_all_findings() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let mut src = String::from("// NOLINT(module_size_distribution)\npackage main\n");
        for i in 0..40 { src.push_str(&format!("func Func{}() {{}}\n", i)); }
        let tree = parser.parse(&src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "test.go");
        assert!(findings.is_empty(), "NOLINT should suppress all findings, got: {:?}", findings);
    }

    #[test]
    fn test_const_block_not_anemic() {
        let src = r#"package app

const (
    StatusOK    = 0
    StatusError = 1
    StatusPending = 2
    StatusCancelled = 3
    StatusDone = 4
)
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings.iter().any(|f| f.pattern == "anemic_module"),
            "const block with multiple specs should not be flagged as anemic_module"
        );
    }

    #[test]
    fn test_var_block_not_anemic() {
        let src = r#"package app

var (
    defaultHost = "localhost"
    defaultPort = 8080
    defaultTimeout = 30
)
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings.iter().any(|f| f.pattern == "anemic_module"),
            "var block with multiple specs should not be flagged as anemic_module"
        );
    }
}

