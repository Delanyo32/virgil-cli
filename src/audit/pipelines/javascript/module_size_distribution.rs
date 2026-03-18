use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_top_level_definitions, is_entry_file};
use crate::language::Language;
use super::primitives::{extract_snippet, find_capture_index};

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_DEFINITION_THRESHOLD: usize = 1;
const ANEMIC_ENTRY_FILES: &[&str] = &["index.js", "index.mjs"];

const JS_DEFINITION_KINDS: &[&str] = &[
    "function_declaration",
    "class_declaration",
    "lexical_declaration",
    "variable_declaration",
    "export_statement",
];

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

/// Check if an export_statement node is a re-export (has a `source` field, i.e. `from '...'`).
fn is_reexport_statement(node: tree_sitter::Node) -> bool {
    // tree-sitter-javascript gives export_statement a `source` field for re-exports
    if node.child_by_field_name("source").is_some() {
        return true;
    }
    // Fallback: check if any child is a string containing the from clause
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            // A string child directly under export_statement indicates `from "..."` clause
            return true;
        }
    }
    false
}

pub struct ModuleSizeDistributionPipeline {
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new() -> Result<Self> {
        let exported_query_str = r#"
[
  (export_statement
    declaration: (_) @decl) @export

  (export_statement
    (export_clause
      (export_specifier) @specifier)) @export_clause
]
"#;
        let exported_query = Query::new(&js_lang(), exported_query_str)
            .with_context(|| "failed to compile exported symbols query for JavaScript architecture")?;

        Ok(Self {
            exported_query: Arc::new(exported_query),
        })
    }

    /// Count top-level definitions. For `export_statement`, count it as 1 definition
    /// (the export wraps a declaration).
    fn count_definitions(&self, root: tree_sitter::Node) -> usize {
        count_top_level_definitions(root, JS_DEFINITION_KINDS)
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

        let total_definitions = self.count_definitions(root);
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
        // Count exported symbols: declarations wrapped in export_statement, plus export specifiers
        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let export_idx = find_capture_index(&self.exported_query, "export");
            let specifier_idx = find_capture_index(&self.exported_query, "specifier");
            let export_clause_idx = find_capture_index(&self.exported_query, "export_clause");
            let decl_idx = find_capture_index(&self.exported_query, "decl");

            let mut matches = cursor.matches(&self.exported_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == export_idx {
                        // export_statement with a declaration child -> 1 exported symbol
                        // Skip re-exports: export statements containing "from" (source field)
                        if is_reexport_statement(cap.node) {
                            continue;
                        }
                        if cap.node.parent().map_or(false, |p| p.kind() == "program") {
                            exported_count += 1;
                        }
                    } else if cap.index as usize == specifier_idx {
                        // Each export_specifier in an export_clause
                        // Skip if parent export_statement is a re-export
                        let is_reexport = {
                            let mut node = cap.node.parent();
                            let mut found = false;
                            while let Some(p) = node {
                                if p.kind() == "export_statement" {
                                    found = is_reexport_statement(p);
                                    break;
                                }
                                node = p.parent();
                            }
                            found
                        };
                        if !is_reexport {
                            exported_count += 1;
                        }
                    } else if cap.index as usize == export_clause_idx {
                        // Already counted via specifiers; don't double-count
                    } else if cap.index as usize == decl_idx {
                        // Already counted via export; don't double-count
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
        if total_definitions == ANEMIC_DEFINITION_THRESHOLD && !is_entry_file(file_path, ANEMIC_ENTRY_FILES) {
            let snippet = {
                let mut cursor = root.walk();
                root.children(&mut cursor)
                    .find(|c| JS_DEFINITION_KINDS.contains(&c.kind()))
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
        parser.set_language(&js_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_oversized_module() {
        let mut src = String::new();
        for i in 0..31 {
            src.push_str(&format!("function func_{}() {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn no_oversized_for_small_module() {
        let src = "function foo() {}\nfunction bar() {}\nclass Baz {}";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "oversized_module"));
    }

    #[test]
    fn detects_monolithic_export() {
        let mut src = String::new();
        for i in 0..21 {
            src.push_str(&format!("export function func_{}() {{}}\n", i));
        }
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "monolithic_export_surface"));
    }

    #[test]
    fn no_monolithic_for_few_exports() {
        let src = "export function foo() {}\nexport function bar() {}";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "monolithic_export_surface"));
    }

    #[test]
    fn detects_anemic_module() {
        let src = "const MAX_RETRIES = 5;";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_entry_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&js_lang()).unwrap();
        let src = "export function main() {}";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "index.js");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = "function foo() {}\nfunction bar() {}";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }
}
