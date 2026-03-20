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

        Ok(Self {
            exported_query: Arc::new(exported_query),
            class_query: Arc::new(class_query),
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
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Pattern 1: excessive_public_api
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

        // Pattern 2: leaky_abstraction_boundary
        // Find classes with public data members (field declarations under public access)
        {
            let mut cursor = QueryCursor::new();
            let class_name_idx = find_capture_index(&self.class_query, "class_name");
            let class_body_idx = find_capture_index(&self.class_query, "class_body");

            let mut matches = cursor.matches(&self.class_query, root, source);
            let mut reported_classes = HashSet::new();

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

                if class_name.is_empty() || reported_classes.contains(class_name) {
                    continue;
                }

                if let Some(body) = body_node
                    && let Some(field_name) = find_public_data_member(body, source)
                {
                    reported_classes.insert(class_name.to_string());
                    findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: class_line,
                            column: 1,
                            severity: "warning".to_string(),
                            pipeline: "api_surface_area".to_string(),
                            pattern: "leaky_abstraction_boundary".to_string(),
                            message: format!(
                                "Class `{}` has public data member `{}` — consider encapsulating with accessor methods",
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

/// Walk the field_declaration_list of a class to find public data members.
/// In tree-sitter-cpp, access_specifier nodes (e.g., `public:`) are siblings
/// to actual member declarations. The default access for `class` is private.
/// We track the current access level and look for field_declaration nodes
/// (non-function data members) under public access.
fn find_public_data_member(body: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Default access for class is private
    let mut current_access = "private";
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        if child.kind() == "access_specifier" {
            // Extract the access keyword: "public", "private", "protected"
            let text = node_text(child, source).trim_end_matches(':').trim();
            current_access = match text {
                "public" => "public",
                "private" => "private",
                "protected" => "protected",
                _ => current_access,
            };
            continue;
        }

        // Only check field_declaration under public access
        if current_access == "public" && child.kind() == "field_declaration" {
            // Check if this is a data member (not a method declaration)
            // Method declarations have a function_declarator child
            if !has_function_declarator(child) {
                // Extract the field name from the declarator
                if let Some(name) = extract_field_name(child, source) {
                    return Some(name);
                }
            }
        }
    }

    None
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
}
