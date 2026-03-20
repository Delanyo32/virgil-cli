use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{find_capture_index, has_storage_class, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_top_level_definitions;
use crate::language::Language;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;
const LEAKY_FIELD_THRESHOLD: usize = 4;

const C_SYMBOL_KINDS: &[&str] = &[
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

pub struct ApiSurfaceAreaPipeline {
    symbol_query: Arc<Query>,
    struct_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        let symbol_query_str = r#"
[
  (function_definition) @sym
  (declaration) @sym
  (struct_specifier) @sym
  (enum_specifier) @sym
  (union_specifier) @sym
  (type_definition) @sym
]
"#;
        let symbol_query = Query::new(&c_lang(), symbol_query_str)
            .with_context(|| "failed to compile symbol query for C API surface")?;

        // Match struct definitions with field lists
        let struct_query_str = r#"
(struct_specifier
  name: (type_identifier) @struct_name
  body: (field_declaration_list) @field_list) @struct_def
"#;
        let struct_query = Query::new(&c_lang(), struct_query_str)
            .with_context(|| "failed to compile struct query for C API surface")?;

        Ok(Self {
            symbol_query: Arc::new(symbol_query),
            struct_query: Arc::new(struct_query),
        })
    }

    /// Count the number of field declarations in a field_declaration_list node.
    fn count_fields(field_list: tree_sitter::Node) -> usize {
        let mut count = 0;
        let mut cursor = field_list.walk();
        for child in field_list.named_children(&mut cursor) {
            if child.kind() == "field_declaration" {
                count += 1;
            }
        }
        count
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
        let total_symbols = count_top_level_definitions(root, C_SYMBOL_KINDS);

        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let sym_idx = find_capture_index(&self.symbol_query, "sym");
            let mut matches = cursor.matches(&self.symbol_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == sym_idx {
                        // Only count top-level symbols
                        if cap
                            .node
                            .parent()
                            .is_some_and(|p| p.kind() == "translation_unit")
                        {
                            // Not static => exported
                            if !has_storage_class(cap.node, source, "static") {
                                exported_count += 1;
                            }
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
        // Non-static struct definitions with fields in header files (.h)
        if file_path.ends_with(".h") {
            let mut cursor = QueryCursor::new();
            let struct_name_idx = find_capture_index(&self.struct_query, "struct_name");
            let field_list_idx = find_capture_index(&self.struct_query, "field_list");
            let struct_def_idx = find_capture_index(&self.struct_query, "struct_def");

            let mut matches = cursor.matches(&self.struct_query, root, source);
            let mut reported_structs = HashSet::new();

            while let Some(m) = matches.next() {
                let mut struct_name = "";
                let mut struct_line = 0u32;
                let mut field_count = 0usize;
                let mut is_static = false;

                for cap in m.captures {
                    if cap.index as usize == struct_name_idx {
                        struct_name = node_text(cap.node, source);
                        struct_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == field_list_idx {
                        field_count = Self::count_fields(cap.node);
                    }
                    if cap.index as usize == struct_def_idx {
                        // Check if the struct definition (or its parent) is static
                        is_static = has_storage_class(cap.node, source, "static");
                        // Also check parent (e.g., if wrapped in a declaration or type_definition)
                        if let Some(parent) = cap.node.parent()
                            && has_storage_class(parent, source, "static") {
                                is_static = true;
                            }
                    }
                }

                if !struct_name.is_empty()
                    && !is_static
                    && field_count >= LEAKY_FIELD_THRESHOLD
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
                            "Struct `{}` exposes {} fields in a header file — consider using an opaque pointer",
                            struct_name, field_count
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

    fn parse_and_check(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::new();
        // 10 non-static + 1 static = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        src.push_str("static void private_func(void) {}\n");
        let findings = parse_and_check(&src, "utils.c");
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
void foo(void) {}
void bar(void) {}
static void baz(void) {}
static void qux(void) {}
"#;
        let findings = parse_and_check(src, "utils.c");
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction_in_header() {
        let src = r#"
struct Connection {
    int socket_fd;
    int buffer_pos;
    int retry_count;
    int max_retries;
    int is_connected;
};
"#;
        let findings = parse_and_check(src, "connection.h");
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_in_c_file() {
        let src = r#"
struct Connection {
    int socket_fd;
    int buffer_pos;
    int retry_count;
    int max_retries;
    int is_connected;
};
"#;
        let findings = parse_and_check(src, "connection.c");
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_small_struct() {
        let src = r#"
struct Point {
    int x;
    int y;
};
"#;
        let findings = parse_and_check(src, "geometry.h");
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_opaque_forward_declaration() {
        let src = r#"
struct Connection;
"#;
        let findings = parse_and_check(src, "connection.h");
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }
}
