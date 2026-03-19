use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_top_level_definitions;
use crate::language::Language;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;

/// Top-level definition kinds for counting total symbols.
/// Includes `export_statement` because `export function foo()` is parsed as
/// an `export_statement` at program level, not as a bare `function_declaration`.
const TS_SYMBOL_KINDS: &[&str] = &[
    "function_declaration",
    "class_declaration",
    "lexical_declaration",
    "variable_declaration",
    "type_alias_declaration",
    "interface_declaration",
    "enum_declaration",
    "export_statement",
];

pub struct ApiSurfaceAreaPipeline {
    _language: Language,
    exported_query: Arc<Query>,
    leaky_class_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();

        // Count exported declarations (export_statement wrapping a declaration)
        let exported_query_str = r#"
(export_statement
  declaration: [
    (function_declaration) @decl
    (class_declaration) @decl
    (lexical_declaration) @decl
    (type_alias_declaration) @decl
    (interface_declaration) @decl
    (enum_declaration) @decl
  ]) @export
"#;
        let exported_query = Query::new(&ts_lang, exported_query_str).with_context(
            || "failed to compile exported symbols query for TypeScript API surface",
        )?;

        // Find exported classes with public_field_definition nodes.
        // In TypeScript's grammar, ALL class fields are `public_field_definition`
        // regardless of access modifier. We capture the accessibility_modifier
        // to filter out private/protected fields in the check() logic.
        let leaky_class_query_str = r#"
(export_statement
  declaration: (class_declaration
    name: (type_identifier) @class_name
    body: (class_body
      (public_field_definition
        (accessibility_modifier)? @access_mod
        name: (property_identifier) @field_name)))) @class_def
"#;
        let leaky_class_query = Query::new(&ts_lang, leaky_class_query_str)
            .with_context(|| "failed to compile leaky class query for TypeScript API surface")?;

        Ok(Self {
            _language: language,
            exported_query: Arc::new(exported_query),
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
        let total_symbols = count_top_level_definitions(root, TS_SYMBOL_KINDS);

        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let export_idx = find_capture_index(&self.exported_query, "export");
            let mut matches = cursor.matches(&self.exported_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == export_idx {
                        // Only count top-level exports
                        if cap.node.parent().map_or(true, |p| p.kind() == "program") {
                            exported_count += 1;
                        }
                    }
                }
            }
        }

        if total_symbols >= EXCESSIVE_API_MIN_SYMBOLS {
            let ratio = exported_count as f64 / total_symbols as f64;
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
                        total_symbols,
                        ratio * 100.0,
                        (EXCESSIVE_API_EXPORT_RATIO * 100.0) as u32
                    ),
                    snippet: String::new(),
                });
            }
        }

        // Pattern 2: leaky_abstraction_boundary
        // Detects exported classes with public field definitions.
        // In TS grammar, all fields are `public_field_definition`; we filter
        // by accessibility_modifier to skip private/protected fields.
        {
            let mut cursor = QueryCursor::new();
            let class_name_idx = find_capture_index(&self.leaky_class_query, "class_name");
            let field_name_idx = find_capture_index(&self.leaky_class_query, "field_name");
            let access_mod_idx = find_capture_index(&self.leaky_class_query, "access_mod");

            let mut matches = cursor.matches(&self.leaky_class_query, root, source);
            let mut reported_classes = HashSet::new();

            while let Some(m) = matches.next() {
                let mut class_name = "";
                let mut class_line = 0u32;
                let mut field_name = "";
                let mut access_modifier = "";

                for cap in m.captures {
                    if cap.index as usize == class_name_idx {
                        class_name = node_text(cap.node, source);
                        class_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == field_name_idx {
                        field_name = node_text(cap.node, source);
                    }
                    if cap.index as usize == access_mod_idx {
                        access_modifier = node_text(cap.node, source);
                    }
                }

                // Skip private and protected fields
                if access_modifier == "private" || access_modifier == "protected" {
                    continue;
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

    fn ts_lang() -> tree_sitter::Language {
        Language::TypeScript.tree_sitter_language()
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
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
    public store: Map<string, Buffer> = new Map();
    public maxSize: number;

    get(key: string): Buffer | undefined { return this.store.get(key); }
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
    public connections: string[] = [];
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
    fn no_leaky_for_private_fields() {
        let src = r#"
export class ConnectionPool {
    private connections: string[] = [];
    private maxSize: number = 10;
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
    fn counts_ts_specific_symbols() {
        // TS-specific: type_alias_declaration, interface_declaration, enum_declaration
        let mut src = String::new();
        for i in 0..4 {
            src.push_str(&format!("export type Type{} = string;\n", i));
        }
        for i in 0..4 {
            src.push_str(&format!("export interface Iface{} {{ x: number; }}\n", i));
        }
        for i in 0..3 {
            src.push_str(&format!("export enum Enum{} {{ A, B }}\n", i));
        }
        // 11 exported out of 11 total = 100% > 80%, and >= 10 symbols
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }
}
