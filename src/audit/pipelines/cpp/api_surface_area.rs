use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{
    compile_struct_specifier_query, find_capture_index, has_storage_class, is_cpp_header,
    is_generated_cpp_file, node_text,
};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_top_level_definitions, is_test_file};
use crate::language::Language;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;

const CPP_SYMBOL_KINDS: &[&str] = &[
    "function_definition",
    "declaration",
    "class_specifier",
    "struct_specifier",
    "enum_specifier",
    "namespace_definition",
    "template_declaration",
    "type_definition",
];

fn cpp_lang() -> tree_sitter::Language {
    Language::Cpp.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    exported_query: Arc<Query>,
    class_query: Arc<Query>,
    struct_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        // Query for top-level definitions (exported = non-static)
        let exported_query_str = r#"
[
  (function_definition) @def
  (declaration) @def
  (class_specifier name: (type_identifier)) @def
  (struct_specifier name: (type_identifier)) @def
  (enum_specifier name: (type_identifier)) @def
  (namespace_definition) @def
  (template_declaration) @def
  (type_definition) @def
]
"#;
        let exported_query = Query::new(&cpp_lang(), exported_query_str)
            .with_context(|| "failed to compile exported query for C++ API surface")?;

        // Query for class specifiers with their body
        let class_query_str = r#"
(class_specifier
  name: (type_identifier) @class_name
  body: (field_declaration_list) @class_body) @class_def
"#;
        let class_query = Query::new(&cpp_lang(), class_query_str)
            .with_context(|| "failed to compile class query for C++ API surface")?;

        let struct_query = compile_struct_specifier_query()
            .with_context(|| "failed to compile struct query for C++ API surface")?;

        Ok(Self {
            exported_query: Arc::new(exported_query),
            class_query: Arc::new(class_query),
            struct_query,
        })
    }
}

impl Pipeline for ApiSurfaceAreaPipeline {
    fn name(&self) -> &str {
        "api_surface_area"
    }

    fn description(&self) -> &str {
        "Detects excessive public API and leaky abstraction boundaries in C++ code"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return vec![];
        }
        if is_generated_cpp_file(file_path, source) {
            return vec![];
        }

        let is_header = is_cpp_header(file_path);
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Pattern 1: excessive_public_api — skip for header files
        if !is_header {
            let total_symbols = count_top_level_definitions(root, CPP_SYMBOL_KINDS);
            let mut exported_count = 0usize;
            {
                let mut cursor = QueryCursor::new();
                let def_idx = find_capture_index(&self.exported_query, "def");
                let mut matches = cursor.matches(&self.exported_query, root, source);
                while let Some(m) = matches.next() {
                    for cap in m.captures {
                        if cap.index as usize == def_idx {
                            let is_top_level = cap
                                .node
                                .parent()
                                .is_some_and(|p| p.kind() == "translation_unit");
                            if is_top_level && !has_storage_class(cap.node, source, "static") {
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
        }

        // Pattern 2: leaky_abstraction_boundary
        // Dedup key: (class_name, start_line) — handles same name in different namespaces.
        let mut reported: HashSet<(String, u32)> = HashSet::new();

        // --- class_specifier pass (default access: private) ---
        {
            let mut cursor = QueryCursor::new();
            let class_name_idx = find_capture_index(&self.class_query, "class_name");
            let class_body_idx = find_capture_index(&self.class_query, "class_body");
            let mut matches = cursor.matches(&self.class_query, root, source);

            while let Some(m) = matches.next() {
                let mut class_name = "";
                let mut class_line = 0u32;
                let mut body_node = None;
                for cap in m.captures {
                    if cap.index as usize == class_name_idx {
                        class_name = node_text(cap.node, source);
                        class_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == class_body_idx {
                        body_node = Some(cap.node);
                    }
                }
                if class_name.is_empty() {
                    continue;
                }
                let key = (class_name.to_string(), class_line);
                if reported.contains(&key) {
                    continue;
                }
                if let Some(body) = body_node {
                    let (count, first_name) = count_public_data_members(body, source, false);
                    if count > 0 {
                        let severity = if count >= 10 {
                            "error"
                        } else if count >= 3 {
                            "warning"
                        } else {
                            "info"
                        };
                        reported.insert(key);
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: class_line,
                            column: 1,
                            severity: severity.to_string(),
                            pipeline: "api_surface_area".to_string(),
                            pattern: "leaky_abstraction_boundary".to_string(),
                            message: format!(
                                "Class `{}` has public data member `{}` — consider encapsulating with accessor methods",
                                class_name,
                                first_name.as_deref().unwrap_or("<unknown>")
                            ),
                            snippet: String::new(),
                        });
                    }
                }
            }
        }

        // --- struct_specifier pass (default access: public) ---
        {
            let mut cursor = QueryCursor::new();
            let struct_name_idx = find_capture_index(&self.struct_query, "struct_name");
            let struct_body_idx = find_capture_index(&self.struct_query, "struct_body");
            let mut matches = cursor.matches(&self.struct_query, root, source);

            while let Some(m) = matches.next() {
                let mut struct_name = "";
                let mut struct_line = 0u32;
                let mut body_node = None;
                for cap in m.captures {
                    if cap.index as usize == struct_name_idx {
                        struct_name = node_text(cap.node, source);
                        struct_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == struct_body_idx {
                        body_node = Some(cap.node);
                    }
                }
                if struct_name.is_empty() {
                    continue;
                }
                let key = (struct_name.to_string(), struct_line);
                if reported.contains(&key) {
                    continue;
                }
                if let Some(body) = body_node {
                    let (count, first_name) = count_public_data_members(body, source, true);
                    if count > 0 {
                        let severity = if count >= 10 {
                            "error"
                        } else if count >= 3 {
                            "warning"
                        } else {
                            "info"
                        };
                        reported.insert(key);
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: struct_line,
                            column: 1,
                            severity: severity.to_string(),
                            pipeline: "api_surface_area".to_string(),
                            pattern: "leaky_abstraction_boundary".to_string(),
                            message: format!(
                                "Struct `{}` has {} public data member(s) — consider encapsulating with accessor methods",
                                struct_name, count
                            ),
                            snippet: String::new(),
                        });
                    }
                }
            }
        }

        findings
    }
}

/// Walk a `field_declaration_list` and count public data members (non-method fields).
/// `default_public`: true for `struct` (default access = public), false for `class` (private).
/// Returns `(count, first_field_name)`.
fn count_public_data_members(
    body: tree_sitter::Node,
    source: &[u8],
    default_public: bool,
) -> (usize, Option<String>) {
    let mut current_access = if default_public { "public" } else { "private" };
    let mut count = 0usize;
    let mut first_name: Option<String> = None;
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        if child.kind() == "access_specifier" {
            let text = node_text(child, source).trim_end_matches(':').trim();
            current_access = match text {
                "public" => "public",
                "private" => "private",
                "protected" => "protected",
                _ => current_access,
            };
            continue;
        }
        if current_access == "public" && child.kind() == "field_declaration" {
            if !has_function_declarator(child) {
                if let Some(name) = extract_field_name(child, source) {
                    if first_name.is_none() {
                        first_name = Some(name);
                    }
                    count += 1;
                }
            }
        }
    }

    (count, first_name)
}

/// Check if a field_declaration contains a function_declarator (i.e., it's a method declaration)
fn has_function_declarator(node: tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_declarator" {
            return true;
        }
        // Check nested declarators (e.g., pointer declarators wrapping function declarators)
        if has_function_declarator(child) {
            return true;
        }
    }
    false
}

/// Extract the name of a field from a field_declaration node
fn extract_field_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if let Some(declarator) = node.child_by_field_name("declarator") {
        return extract_identifier_from_declarator(declarator, source);
    }
    None
}

/// Recursively extract an identifier name from a declarator node
fn extract_identifier_from_declarator(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if node.kind() == "field_identifier" || node.kind() == "identifier" {
        return node.utf8_text(source).ok().map(|s| s.to_string());
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return extract_identifier_from_declarator(inner, source);
    }
    // Walk children as fallback
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "field_identifier" || child.kind() == "identifier" {
            return child.utf8_text(source).ok().map(|s| s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&cpp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&cpp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::new();
        // 10 non-static + 1 static = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("void func_{}() {{}}\n", i));
        }
        src.push_str("static void private_func() {}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
void foo() {}
void bar() {}
static void baz() {}
static void qux() {}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"
class HttpClient {
public:
    std::string base_url;
    int timeout_ms;
    void get();
};
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
class HttpClient {
public:
    void get();
    void post();
private:
    std::string base_url;
    int timeout_ms;
};
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_default_private_class() {
        // Default access for class is private
        let src = r#"
class InternalPool {
    std::string name;
    int size;
public:
    void init();
};
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_public_methods_only() {
        let src = r#"
class Service {
public:
    void start();
    void stop();
    int status();
};
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn test_struct_with_public_fields_detected() {
        // structs default to public — all fields are public by default
        let src = r#"
struct Config {
    std::string host;
    int port;
};
"#;
        let findings = parse_and_check_file(src, "config.cpp");
        assert!(
            findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "Struct with public fields should trigger leaky_abstraction_boundary"
        );
    }

    #[test]
    fn test_header_file_excessive_api_suppressed() {
        // .hpp with 15 declarations — excessive_public_api skipped for headers
        let mut src = String::new();
        for i in 0..15 {
            src.push_str(&format!("void func_{}(int x);\n", i));
        }
        let findings = parse_and_check_file(&src, "api.hpp");
        assert!(
            !findings.iter().any(|f| f.pattern == "excessive_public_api"),
            "Header files should not trigger excessive_public_api"
        );
    }

    #[test]
    fn test_generated_protobuf_excluded() {
        let src = r#"
class UserMessage {
public:
    int id;
    std::string name;
};
"#;
        let findings = parse_and_check_file(src, "user.pb.h");
        assert!(findings.is_empty(), "Protobuf header should produce 0 findings");
    }

    #[test]
    fn test_two_classes_same_name_different_namespaces_both_reported() {
        let src = r#"
namespace A {
class Foo {
public:
    int x;
};
}
namespace B {
class Foo {
public:
    int y;
};
}
"#;
        let findings = parse_and_check_file(src, "multi_ns.cpp");
        let count = findings
            .iter()
            .filter(|f| f.pattern == "leaky_abstraction_boundary")
            .count();
        assert_eq!(
            count, 2,
            "Two classes named Foo in different namespaces should each be reported"
        );
    }

    #[test]
    fn test_severity_escalation_many_public_fields() {
        // Class with 12 public data members → severity "error"
        let mut src = String::from("class BigData {\npublic:\n");
        for i in 0..12 {
            src.push_str(&format!("    int field_{};\n", i));
        }
        src.push_str("};\n");
        let findings = parse_and_check_file(&src, "bigdata.cpp");
        let f = findings
            .iter()
            .find(|f| f.pattern == "leaky_abstraction_boundary")
            .expect("12 public data members should trigger leaky_abstraction_boundary");
        assert_eq!(f.severity, "error", "12 public members should be 'error' severity");
    }

    #[test]
    fn test_test_file_excluded() {
        let mut src = String::new();
        for i in 0..15 {
            src.push_str(&format!("void func_{}() {{}}\n", i));
        }
        let findings = parse_and_check_file(&src, "connection_test.cpp");
        assert!(
            findings.is_empty(),
            "Test files should produce 0 findings, got: {:?}",
            findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
        );
    }
}
