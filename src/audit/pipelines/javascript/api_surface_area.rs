use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::find_capture_index;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_top_level_definitions;
use crate::language::Language;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;

const JS_SYMBOL_KINDS: &[&str] = &[
    "function_declaration",
    "class_declaration",
    "lexical_declaration",
    "variable_declaration",
];

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    exported_decl_query: Arc<Query>,
    leaky_class_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        // Match exported declarations (export_statement wrapping a declaration)
        let exported_decl_query_str = r#"
(export_statement
  declaration: [
    (function_declaration) @decl
    (class_declaration) @decl
    (lexical_declaration) @decl
    (variable_declaration) @decl
  ]) @export
"#;
        let exported_decl_query = Query::new(&js_lang(), exported_decl_query_str).with_context(
            || "failed to compile exported declaration query for JavaScript API surface",
        )?;

        // Match exported classes with field definitions (leaky abstraction)
        // In JS, class fields without a private marker (#) default to public
        let leaky_class_query_str = r#"
(export_statement
  declaration: (class_declaration
    name: (identifier) @class_name
    body: (class_body
      (field_definition
        property: (property_identifier) @field_name)))) @export_class
"#;
        let leaky_class_query = Query::new(&js_lang(), leaky_class_query_str)
            .with_context(|| "failed to compile leaky class query for JavaScript API surface")?;

        Ok(Self {
            exported_decl_query: Arc::new(exported_decl_query),
            leaky_class_query: Arc::new(leaky_class_query),
        })
    }
}

impl Pipeline for ApiSurfaceAreaPipeline {
    fn name(&self) -> &str {
        "api_surface_area"
    }

    fn description(&self) -> &str {
        "Detects excessive public API and leaky abstraction boundaries"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Pattern 1: excessive_public_api
        // Count all top-level declarations (both exported and non-exported)
        let total_symbols = count_top_level_definitions(root, JS_SYMBOL_KINDS);

        // Also count export_statement nodes that wrap declarations as part of total
        let mut export_with_decl_count = 0usize;
        {
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                if child.kind() == "export_statement" {
                    // Check if this export wraps a declaration
                    if child.child_by_field_name("declaration").is_some() {
                        export_with_decl_count += 1;
                    }
                }
            }
        }

        // Total = bare declarations + exported declarations
        let total = total_symbols + export_with_decl_count;

        // Count exported declarations
        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let export_idx = find_capture_index(&self.exported_decl_query, "export");
            let mut matches = cursor.matches(&self.exported_decl_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == export_idx
                        && cap.node.parent().is_some_and(|p| p.kind() == "program") {
                            exported_count += 1;
                        }
                }
            }
        }

        if total >= EXCESSIVE_API_MIN_SYMBOLS {
            let ratio = exported_count as f64 / total as f64;
            if ratio > EXCESSIVE_API_EXPORT_RATIO {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: 1,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "api_surface_area".to_string(),
                    pattern: "excessive_public_api".to_string(),
                    message: format!(
                        "Module exports {}/{} symbols ({:.0}% exported, threshold: >{}%)",
                        exported_count,
                        total,
                        ratio * 100.0,
                        (EXCESSIVE_API_EXPORT_RATIO * 100.0) as u32
                    ),
                    snippet: String::new(),
                });
            }
        }

        // Pattern 2: leaky_abstraction_boundary
        // Find exported classes with field definitions (JS fields default to public)
        {
            let mut cursor = QueryCursor::new();
            let class_name_idx = find_capture_index(&self.leaky_class_query, "class_name");
            let field_name_idx = find_capture_index(&self.leaky_class_query, "field_name");

            let mut matches = cursor.matches(&self.leaky_class_query, root, source);
            let mut reported_classes = HashSet::new();

            while let Some(m) = matches.next() {
                let mut class_name = "";
                let mut class_line = 0u32;
                let mut field_name = "";

                for cap in m.captures {
                    if cap.index as usize == class_name_idx {
                        class_name = cap.node.utf8_text(source).unwrap_or("");
                        class_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == field_name_idx {
                        field_name = cap.node.utf8_text(source).unwrap_or("");
                    }
                }

                if !class_name.is_empty() && !reported_classes.contains(class_name) {
                    reported_classes.insert(class_name.to_string());
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: class_line,
                        column: 1,
                        severity: "warning".to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "leaky_abstraction_boundary".to_string(),
                        message: format!(
                            "Exported class `{}` has public field `{}` — consider encapsulating with methods",
                            class_name, field_name
                        ),
                        snippet: String::new(),
                    });
                }
            }
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
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::new();
        // 10 exported + 1 private = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("export function func_{}() {{}}\n", i));
        }
        src.push_str("function private_func() {}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
export function foo() {}
export function bar() {}
function baz() {}
function qux() {}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"
export class CacheManager {
    store = new Map();
    keys = [];
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_non_exported_class() {
        let src = r#"
class InternalPool {
    connections = [];
    maxSize = 10;
}
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_class_without_fields() {
        let src = r#"
export class Service {
    getData() { return []; }
    process() { return true; }
}
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }
}
