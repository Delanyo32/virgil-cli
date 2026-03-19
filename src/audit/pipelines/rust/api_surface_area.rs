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

const RUST_SYMBOL_KINDS: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "type_item",
    "const_item",
    "static_item",
    "macro_definition",
];

fn rust_lang() -> tree_sitter::Language {
    Language::Rust.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    pub_symbol_query: Arc<Query>,
    leaky_struct_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        let pub_symbol_query_str = r#"
[
  (function_item (visibility_modifier) @vis) @pub_sym
  (struct_item (visibility_modifier) @vis) @pub_sym
  (enum_item (visibility_modifier) @vis) @pub_sym
  (trait_item (visibility_modifier) @vis) @pub_sym
  (type_item (visibility_modifier) @vis) @pub_sym
  (const_item (visibility_modifier) @vis) @pub_sym
  (static_item (visibility_modifier) @vis) @pub_sym
]
"#;
        let pub_symbol_query = Query::new(&rust_lang(), pub_symbol_query_str)
            .with_context(|| "failed to compile pub symbol query for Rust API surface")?;

        let leaky_struct_query_str = r#"
(struct_item
  (visibility_modifier) @struct_vis
  name: (type_identifier) @struct_name
  body: (field_declaration_list
    (field_declaration
      (visibility_modifier) @field_vis
      name: (field_identifier) @field_name))) @struct_def
"#;
        let leaky_struct_query = Query::new(&rust_lang(), leaky_struct_query_str)
            .with_context(|| "failed to compile leaky struct query for Rust API surface")?;

        Ok(Self {
            pub_symbol_query: Arc::new(pub_symbol_query),
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
        let total_symbols = count_top_level_definitions(root, RUST_SYMBOL_KINDS);

        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let pub_sym_idx = find_capture_index(&self.pub_symbol_query, "pub_sym");
            let mut matches = cursor.matches(&self.pub_symbol_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == pub_sym_idx {
                        if cap
                            .node
                            .parent()
                            .map_or(false, |p| p.kind() == "source_file")
                        {
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
        {
            let mut cursor = QueryCursor::new();
            let struct_name_idx = find_capture_index(&self.leaky_struct_query, "struct_name");
            let field_name_idx = find_capture_index(&self.leaky_struct_query, "field_name");
            let _struct_def_idx = find_capture_index(&self.leaky_struct_query, "struct_def");

            let mut matches = cursor.matches(&self.leaky_struct_query, root, source);
            let mut reported_structs = std::collections::HashSet::new();

            while let Some(m) = matches.next() {
                let mut struct_name = "";
                let mut struct_line = 0u32;
                let mut field_name = "";

                for cap in m.captures {
                    if cap.index as usize == struct_name_idx {
                        struct_name = node_text(cap.node, source);
                        struct_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == field_name_idx {
                        field_name = node_text(cap.node, source);
                    }
                }

                if !struct_name.is_empty() && !reported_structs.contains(struct_name) {
                    reported_structs.insert(struct_name.to_string());
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: struct_line,
                        column: 1,
                        severity: "warning".to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "leaky_abstraction_boundary".to_string(),
                        message: format!(
                            "Public struct `{}` has public field `{}` — consider encapsulating with methods",
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
        parser.set_language(&rust_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::new();
        // 10 pub + 1 private = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("pub fn func_{}() {{}}\n", i));
        }
        src.push_str("fn private_func() {}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
pub fn foo() {}
pub fn bar() {}
fn baz() {}
fn qux() {}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"
pub struct ConnectionPool {
    pub connections: Vec<i32>,
    pub max_size: usize,
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
    fn no_leaky_for_private_fields() {
        let src = r#"
pub struct ConnectionPool {
    connections: Vec<i32>,
    max_size: usize,
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
    fn no_leaky_for_private_struct() {
        let src = r#"
struct InternalPool {
    pub connections: Vec<i32>,
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
