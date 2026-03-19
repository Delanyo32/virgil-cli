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

const GO_SYMBOL_KINDS: &[&str] = &[
    "function_declaration",
    "method_declaration",
    "type_declaration",
    "var_declaration",
    "const_declaration",
];

fn go_lang() -> tree_sitter::Language {
    Language::Go.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    symbol_query: Arc<Query>,
    leaky_struct_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        // Query to find named declarations and check if exported (uppercase)
        let symbol_query_str = r#"
[
  (function_declaration name: (identifier) @name) @def
  (method_declaration name: (field_identifier) @name) @def
  (type_declaration (type_spec name: (type_identifier) @name)) @def
  (var_declaration (var_spec name: (identifier) @name)) @def
  (const_declaration (const_spec name: (identifier) @name)) @def
]
"#;
        let symbol_query = Query::new(&go_lang(), symbol_query_str)
            .with_context(|| "failed to compile symbol query for Go API surface")?;

        // Query to find exported structs with exported fields
        let leaky_struct_query_str = r#"
(type_declaration
  (type_spec
    name: (type_identifier) @struct_name
    type: (struct_type
      (field_declaration_list
        (field_declaration
          name: (field_identifier) @field_name))))) @struct_def
"#;
        let leaky_struct_query = Query::new(&go_lang(), leaky_struct_query_str)
            .with_context(|| "failed to compile leaky struct query for Go API surface")?;

        Ok(Self {
            symbol_query: Arc::new(symbol_query),
            leaky_struct_query: Arc::new(leaky_struct_query),
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
        let total_symbols = count_top_level_definitions(root, GO_SYMBOL_KINDS);

        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let name_idx = find_capture_index(&self.symbol_query, "name");
            let def_idx = find_capture_index(&self.symbol_query, "def");
            let mut matches = cursor.matches(&self.symbol_query, root, source);
            while let Some(m) = matches.next() {
                let mut is_top_level = false;
                let mut is_exported = false;

                for cap in m.captures {
                    if cap.index as usize == def_idx {
                        if cap
                            .node
                            .parent()
                            .map_or(false, |p| p.kind() == "source_file")
                        {
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
        // Exported struct (uppercase name) with exported fields (uppercase field name)
        {
            let mut cursor = QueryCursor::new();
            let struct_name_idx = find_capture_index(&self.leaky_struct_query, "struct_name");
            let field_name_idx = find_capture_index(&self.leaky_struct_query, "field_name");

            let mut matches = cursor.matches(&self.leaky_struct_query, root, source);
            let mut reported_structs = HashSet::new();

            while let Some(m) = matches.next() {
                let mut struct_name = "";
                let mut struct_line = 0u32;
                let mut field_name = "";
                let mut struct_exported = false;
                let mut field_exported = false;

                for cap in m.captures {
                    if cap.index as usize == struct_name_idx {
                        struct_name = node_text(cap.node, source);
                        struct_line = cap.node.start_position().row as u32 + 1;
                        struct_exported = struct_name.starts_with(|c: char| c.is_ascii_uppercase());
                    }
                    if cap.index as usize == field_name_idx {
                        field_name = node_text(cap.node, source);
                        field_exported = field_name.starts_with(|c: char| c.is_ascii_uppercase());
                    }
                }

                if struct_exported
                    && field_exported
                    && !struct_name.is_empty()
                    && !reported_structs.contains(struct_name)
                {
                    reported_structs.insert(struct_name.to_string());
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: struct_line,
                        column: 1,
                        severity: "warning".to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "leaky_abstraction_boundary".to_string(),
                        message: format!(
                            "Exported struct `{}` has exported field `{}` — consider encapsulating with methods",
                            struct_name, field_name
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
        parser.set_language(&go_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::from("package main\n");
        // 10 exported + 1 unexported = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("func Func{}() {{}}\n", i));
        }
        src.push_str("func privateFunc() {}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"package main

func Foo() {}
func Bar() {}
func baz() {}
func qux() {}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"package cache

type Store struct {
    Entries map[string][]byte
    MaxSize int
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
    fn no_leaky_for_unexported_fields() {
        let src = r#"package cache

type Store struct {
    entries map[string][]byte
    maxSize int
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
    fn no_leaky_for_unexported_struct() {
        let src = r#"package cache

type internalStore struct {
    Entries map[string][]byte
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
