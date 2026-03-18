use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::is_entry_file;
use crate::language::Language;
use super::primitives::{extract_snippet, find_capture_index, has_storage_class};

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
/// A forward declaration is a `declaration` node with no `compound_statement` child
/// (no function body) and no `init_declarator` child (no initializer).
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
        // For `declaration` nodes, check if it's a forward declaration
        if child.kind() == "declaration" && is_forward_declaration(child) {
            continue;
        }
        count += 1;
    }
    count
}

/// Check if a `declaration` node is a forward declaration (no body/initializer).
/// A forward declaration has no `compound_statement` child (no function body)
/// and no `init_declarator` child (no variable initializer).
fn is_forward_declaration(decl: tree_sitter::Node) -> bool {
    let mut cursor = decl.walk();
    for child in decl.children(&mut cursor) {
        let kind = child.kind();
        if kind == "compound_statement" || kind == "init_declarator" {
            return false;
        }
    }
    true
}

pub struct ModuleSizeDistributionPipeline {
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        // Match top-level definitions that are NOT static.
        // We query all top-level definitions and then filter out static ones in code.
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
        let mut findings = Vec::new();
        let root = tree.root_node();

        let total_definitions = count_top_level_definitions_excluding_forward_decls(root, C_DEFINITION_KINDS);
        let total_lines = source.split(|&b| b == b'\n').count();

        // Pattern 1: Oversized module
        if total_definitions >= OVERSIZED_SYMBOL_THRESHOLD || total_lines >= OVERSIZED_LINE_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
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
        // Count non-static top-level definitions (exported symbols in C)
        let mut exported_count = 0usize;
        let mut cursor = QueryCursor::new();
        let def_idx = find_capture_index(&self.exported_query, "def");
        let mut matches = cursor.matches(&self.exported_query, root, source);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == def_idx {
                    // Only count top-level symbols
                    if cap.node.parent().map_or(false, |p| p.kind() == "translation_unit") {
                        // Not static => exported
                        if !has_storage_class(cap.node, source, "static") {
                            exported_count += 1;
                        }
                    }
                }
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
                message: "Module contains only 1 definition — consider merging into a related module".to_string(),
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
}
