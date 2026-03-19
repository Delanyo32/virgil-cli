use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_top_level_definitions, is_entry_file};
use crate::language::Language;

const OVERSIZED_SYMBOL_THRESHOLD: usize = 30;
const OVERSIZED_LINE_THRESHOLD: usize = 1000;
const MONOLITHIC_EXPORT_THRESHOLD: usize = 20;
const ANEMIC_MIN_DEFINITIONS: usize = 1;
const ANEMIC_ENTRY_FILES: &[&str] = &["index.ts", "index.tsx"];

const TS_DEFINITION_KINDS: &[&str] = &[
    "function_declaration",
    "class_declaration",
    "lexical_declaration",
    "variable_declaration",
    "type_alias_declaration",
    "interface_declaration",
    "enum_declaration",
    "export_statement",
];

/// Check if an export_statement node is a re-export (has a `source` field, i.e. `from '...'`).
fn is_reexport_statement(node: tree_sitter::Node) -> bool {
    // tree-sitter gives export_statement a `source` field for re-exports
    if node.child_by_field_name("source").is_some() {
        return true;
    }
    // Fallback: check if any child is a string containing the from clause
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            return true;
        }
    }
    false
}

pub struct ModuleSizeDistributionPipeline {
    _language: Language,
    exported_query: Arc<Query>,
}

impl ModuleSizeDistributionPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();

        // Match exported declarations (export_statement wrapping a declaration)
        // and re-export specifiers (export { ... } from '...')
        let exported_query_str = r#"
[
  (export_statement
    declaration: [
      (function_declaration) @decl
      (class_declaration) @decl
      (lexical_declaration) @decl
      (type_alias_declaration) @decl
      (interface_declaration) @decl
      (enum_declaration) @decl
    ]) @export

  (export_statement
    (export_clause
      (export_specifier) @specifier)) @reexport
]
"#;
        let exported_query = Query::new(&ts_lang, exported_query_str).with_context(
            || "failed to compile exported symbols query for TypeScript architecture",
        )?;

        Ok(Self {
            _language: language,
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

        let total_definitions = count_top_level_definitions(root, TS_DEFINITION_KINDS);
        let total_lines = source.split(|&b| b == b'\n').count();

        // Pattern 1: Oversized module
        if total_definitions >= OVERSIZED_SYMBOL_THRESHOLD
            || total_lines >= OVERSIZED_LINE_THRESHOLD
        {
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
        let mut exported_count = 0usize;
        let mut cursor = QueryCursor::new();
        let export_idx = find_capture_index(&self.exported_query, "export");
        let specifier_idx = find_capture_index(&self.exported_query, "specifier");
        let reexport_idx = find_capture_index(&self.exported_query, "reexport");
        let mut matches = cursor.matches(&self.exported_query, root, source);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == export_idx {
                    // Direct export declaration — count as 1 exported symbol
                    // Skip re-exports: export statements containing "from" (source field)
                    if is_reexport_statement(cap.node) {
                        continue;
                    }
                    if cap.node.parent().map_or(true, |p| p.kind() == "program") {
                        exported_count += 1;
                    }
                } else if cap.index as usize == specifier_idx {
                    // Each export specifier in `export { A, B } from '...'`
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
                } else if cap.index as usize == reexport_idx {
                    // Don't double-count — specifiers handle the individual symbols
                    let _ = cap;
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
        if total_definitions == ANEMIC_MIN_DEFINITIONS
            && !is_entry_file(file_path, ANEMIC_ENTRY_FILES)
        {
            let snippet = {
                let mut cursor = root.walk();
                root.children(&mut cursor)
                    .find(|c| TS_DEFINITION_KINDS.contains(&c.kind()))
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

    fn ts_lang() -> tree_sitter::Language {
        Language::TypeScript.tree_sitter_language()
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
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
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "monolithic_export_surface")
        );
    }

    #[test]
    fn detects_anemic_module() {
        let src = "export const VERSION: string = \"1.0.0\";";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_entry_files() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang()).unwrap();
        let src = "export function main() {}";
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ModuleSizeDistributionPipeline::new(Language::TypeScript).unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "index.ts");
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn no_anemic_for_multiple_definitions() {
        let src = "function foo() {}\nfunction bar() {}";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }

    #[test]
    fn counts_ts_specific_definitions() {
        // type_alias_declaration, interface_declaration, enum_declaration are TS-specific
        let src = "type Foo = string;\ninterface Bar { x: number; }\nenum Color { Red, Green }";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "anemic_module"));
    }
}
