use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    count_top_level_definitions, is_generated_go_file, is_nolint_suppressed, is_test_file,
};
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
        // Skip test files and generated files
        if is_test_file(file_path) || is_generated_go_file(file_path, source) {
            return Vec::new();
        }

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
                    if cap.index as usize == def_idx
                        && cap.node.parent().is_some_and(|p| p.kind() == "source_file")
                    {
                        is_top_level = true;
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
                // Check NOLINT on the package declaration
                let suppressed = {
                    let mut c = root.walk();
                    root.children(&mut c)
                        .find(|n| n.kind() == "package_clause")
                        .is_some_and(|pkg| is_nolint_suppressed(source, pkg, self.name()))
                };
                if !suppressed {
                    let severity = if ratio > 0.9 { "warning" } else { "info" };
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: 1,
                        column: 1,
                        severity: severity.to_string(),
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
        }

        // Pattern 2: leaky_abstraction_boundary
        // Collect all exported fields per struct, then emit one finding per struct.
        // Structs with json/yaml/xml tags are serializable by convention — skip them.
        // NOLINT on the struct declaration line suppresses the finding.
        {
            let mut cursor = QueryCursor::new();
            let struct_name_idx = find_capture_index(&self.leaky_struct_query, "struct_name");
            let field_name_idx = find_capture_index(&self.leaky_struct_query, "field_name");
            let struct_def_idx = find_capture_index(&self.leaky_struct_query, "struct_def");

            // Key: struct_name -> (line, Vec<exported_field_names>, start_byte, end_byte, nolint)
            let mut struct_fields: std::collections::HashMap<
                String,
                (u32, Vec<String>, usize, usize, bool),
            > = std::collections::HashMap::new();

            let mut matches = cursor.matches(&self.leaky_struct_query, root, source);
            while let Some(m) = matches.next() {
                let mut struct_name = "";
                let mut struct_line = 0u32;
                let mut field_name = "";
                let mut struct_exported = false;
                let mut field_exported = false;
                let mut def_start = 0usize;
                let mut def_end = 0usize;

                for cap in m.captures {
                    if cap.index as usize == struct_def_idx {
                        def_start = cap.node.start_byte();
                        def_end = cap.node.end_byte();
                    }
                    if cap.index as usize == struct_name_idx {
                        struct_name = node_text(cap.node, source);
                        struct_line = cap.node.start_position().row as u32 + 1;
                        struct_exported =
                            struct_name.starts_with(|c: char| c.is_ascii_uppercase());
                    }
                    if cap.index as usize == field_name_idx {
                        field_name = node_text(cap.node, source);
                        field_exported =
                            field_name.starts_with(|c: char| c.is_ascii_uppercase());
                    }
                }

                if struct_exported && field_exported && !struct_name.is_empty() {
                    let entry = struct_fields
                        .entry(struct_name.to_string())
                        .or_insert_with(|| {
                            // Check NOLINT on the struct declaration line (and the line above it)
                            let source_str = std::str::from_utf8(source).unwrap_or("");
                            let row = (struct_line as usize).saturating_sub(1); // 0-indexed
                            let nolint = [
                                source_str.lines().nth(row),
                                if row > 0 { source_str.lines().nth(row - 1) } else { None },
                            ]
                            .into_iter()
                            .flatten()
                            .any(|l| {
                                if let Some(pos) = l.find("NOLINT") {
                                    let after = &l[pos + 6..];
                                    if after.starts_with('(') {
                                        after.find(')').is_some_and(|end| {
                                            after[1..end]
                                                .split(',')
                                                .any(|n| n.trim() == "api_surface_area")
                                        })
                                    } else {
                                        true
                                    }
                                } else {
                                    false
                                }
                            });
                            (struct_line, Vec::new(), def_start, def_end, nolint)
                        });
                    entry.1.push(field_name.to_string());
                }
            }

            for (struct_name, (line, fields, start_byte, end_byte, nolint)) in struct_fields {
                if nolint {
                    continue;
                }
                // Skip serializable types (json/yaml/xml tags indicate intentional exported fields)
                let is_serializable = source
                    .get(start_byte..end_byte)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .map(|s| {
                        s.contains("json:\"") || s.contains("yaml:\"") || s.contains("xml:\"")
                    })
                    .unwrap_or(false);
                if is_serializable {
                    continue;
                }

                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: "api_surface_area".to_string(),
                    pattern: "leaky_abstraction_boundary".to_string(),
                    message: format!(
                        "Exported struct `{}` exposes {} field(s) directly: {} — consider encapsulating with methods",
                        struct_name,
                        fields.len(),
                        fields.join(", ")
                    ),
                    snippet: String::new(),
                });
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

    // Task 6: File exclusions
    #[test]
    fn test_test_file_not_analyzed() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let src = r#"package mypackage_test

type TestCase struct {
    Input    string
    Expected string
    Name     string
}
"#;
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "handler_test.go");
        assert!(findings.is_empty(), "test file should produce no findings, got: {:?}", findings);
    }

    #[test]
    fn test_generated_protobuf_not_analyzed() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let mut src = String::from("// Code generated by protoc-gen-go. DO NOT EDIT.\npackage proto\n");
        for i in 0..15 { src.push_str(&format!("func Func{}() {{}}\n", i)); }
        let tree = parser.parse(&src, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "service.pb.go");
        assert!(findings.is_empty(), "generated protobuf file should produce no findings, got: {:?}", findings);
    }

    // Task 7: All exported fields reported + serialization awareness
    #[test]
    fn test_all_exported_fields_reported() {
        let src = r#"package cache

type Store struct {
    Entries  map[string][]byte
    MaxSize  int
    TTL      int
    Hits     int
    Misses   int
}
"#;
        let findings = parse_and_check(src);
        let f = findings.iter().find(|f| f.pattern == "leaky_abstraction_boundary");
        assert!(f.is_some(), "should detect leaky_abstraction_boundary");
        let msg = &f.unwrap().message;
        assert!(msg.contains("Entries"), "message should mention Entries");
        assert!(msg.contains("MaxSize"), "message should mention MaxSize");
        assert!(msg.contains("TTL"), "message should mention TTL");
        assert!(msg.contains("Hits"), "message should mention Hits");
        assert!(msg.contains("Misses"), "message should mention Misses");
    }

    #[test]
    fn test_json_serializable_struct_not_flagged() {
        let src = r#"package api

type UserResponse struct {
    ID    int    `json:"id"`
    Name  string `json:"name"`
    Email string `json:"email"`
}
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "struct with json tags should not be flagged as leaky_abstraction_boundary"
        );
    }

    // Task 8: Severity graduation
    #[test]
    fn test_exactly_10_symbols_80_percent_not_flagged() {
        let mut src = String::from("package main\n");
        for i in 0..8 { src.push_str(&format!("func Func{}() {{}}\n", i)); }
        src.push_str("func private1() {}\nfunc private2() {}\n");
        let findings = parse_and_check(&src);
        assert!(
            !findings.iter().any(|f| f.pattern == "excessive_public_api"),
            "8/10 = exactly 80% should NOT be flagged (threshold is strictly > 80%)"
        );
    }

    #[test]
    fn test_above_90_percent_is_warning() {
        let mut src = String::from("package main\n");
        for i in 0..10 { src.push_str(&format!("func Func{}() {{}}\n", i)); }
        let findings = parse_and_check(&src);
        let f = findings.iter().find(|f| f.pattern == "excessive_public_api");
        assert!(f.is_some(), "100% exported with 10 symbols should trigger excessive_public_api");
        assert_eq!(f.unwrap().severity, "warning", ">90% exported should be severity 'warning'");
    }

    // Task 9: NOLINT suppression
    #[test]
    fn test_nolint_suppresses_excessive_public_api() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let mut src = String::from("// NOLINT(api_surface_area)\npackage main\n");
        for i in 0..10 { src.push_str(&format!("func Func{}() {{}}\n", i)); }
        let tree = parser.parse(&src, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "test.go");
        assert!(
            !findings.iter().any(|f| f.pattern == "excessive_public_api"),
            "NOLINT on package clause should suppress excessive_public_api"
        );
    }

    #[test]
    fn test_nolint_suppresses_leaky_abstraction() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let src = r#"package cache

// NOLINT(api_surface_area)
type Store struct {
    Entries map[string][]byte
    MaxSize int
}
"#;
        let tree = parser.parse(src, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        let findings = pipeline.check(&tree, src.as_bytes(), "store.go");
        assert!(
            !findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "NOLINT above struct should suppress leaky_abstraction_boundary"
        );
    }
}
