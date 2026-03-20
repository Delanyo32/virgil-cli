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

const PHP_SYMBOL_KINDS: &[&str] = &[
    "function_definition",
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
    "enum_declaration",
];

fn php_lang() -> tree_sitter::Language {
    Language::Php.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    method_query: Arc<Query>,
    leaky_property_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        // Match method declarations inside classes to count public vs total members.
        // In PHP, methods without visibility modifier default to public.
        let method_query_str = r#"
(class_declaration
  name: (name) @class_name
  body: (declaration_list
    (method_declaration
      name: (name) @method_name) @method)) @class
"#;
        let method_query = Query::new(&php_lang(), method_query_str)
            .with_context(|| "failed to compile method query for PHP API surface")?;

        // Match public property declarations inside classes (leaky abstraction).
        // We look for property_declaration with a visibility_modifier child.
        let leaky_property_query_str = r#"
(class_declaration
  name: (name) @class_name
  body: (declaration_list
    (property_declaration
      (visibility_modifier) @vis
      (property_element
        (variable_name) @prop_name)))) @class_def
"#;
        let leaky_property_query = Query::new(&php_lang(), leaky_property_query_str)
            .with_context(|| "failed to compile leaky property query for PHP API surface")?;

        Ok(Self {
            method_query: Arc::new(method_query),
            leaky_property_query: Arc::new(leaky_property_query),
        })
    }

    /// Check if a method_declaration has a non-public visibility modifier.
    fn is_non_public_method(method_node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = method_node.walk();
        for child in method_node.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                let text = child.utf8_text(source).unwrap_or("");
                return text == "private" || text == "protected";
            }
        }
        // No visibility modifier means public by default in PHP
        false
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
        // Count top-level symbols (all are exported in PHP)
        let total_symbols = count_top_level_definitions(root, PHP_SYMBOL_KINDS);

        // For methods inside classes, count public vs total
        let mut total_members = total_symbols;
        let mut exported_members = total_symbols; // all top-level PHP symbols are exported

        {
            let mut cursor = QueryCursor::new();
            let method_idx = find_capture_index(&self.method_query, "method");
            let mut matches = cursor.matches(&self.method_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == method_idx {
                        total_members += 1;
                        if !Self::is_non_public_method(cap.node, source) {
                            exported_members += 1;
                        }
                    }
                }
            }
        }

        if total_members >= EXCESSIVE_API_MIN_SYMBOLS {
            let ratio = exported_members as f64 / total_members as f64;
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
                        exported_members,
                        total_members,
                        ratio * 100.0,
                        (EXCESSIVE_API_EXPORT_RATIO * 100.0) as u32
                    ),
                    snippet: String::new(),
                });
            }
        }

        // Pattern 2: leaky_abstraction_boundary
        // Classes with public non-readonly properties
        {
            let mut cursor = QueryCursor::new();
            let class_name_idx = find_capture_index(&self.leaky_property_query, "class_name");
            let vis_idx = find_capture_index(&self.leaky_property_query, "vis");
            let prop_name_idx = find_capture_index(&self.leaky_property_query, "prop_name");

            let mut matches = cursor.matches(&self.leaky_property_query, root, source);
            let mut reported_classes: HashSet<String> = HashSet::new();

            while let Some(m) = matches.next() {
                let mut class_name = "";
                let mut class_line = 0u32;
                let mut vis_text = "";
                let mut prop_name = "";

                for cap in m.captures {
                    if cap.index as usize == class_name_idx {
                        class_name = node_text(cap.node, source);
                        class_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == vis_idx {
                        vis_text = node_text(cap.node, source);
                    }
                    if cap.index as usize == prop_name_idx {
                        prop_name = node_text(cap.node, source);
                    }
                }

                // Only flag public properties (not private or protected)
                if vis_text == "public" && !class_name.is_empty() {
                    // Check if the property_declaration has a readonly modifier
                    // Walk up to find the property_declaration parent of the vis node
                    let is_readonly = is_property_readonly(m, source);

                    if !is_readonly && !reported_classes.contains(class_name) {
                        reported_classes.insert(class_name.to_string());
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: class_line,
                            column: 1,
                            severity: "warning".to_string(),
                            pipeline: "api_surface_area".to_string(),
                            pattern: "leaky_abstraction_boundary".to_string(),
                            message: format!(
                                "Class `{}` has public property `{}` — consider encapsulating with methods",
                                class_name, prop_name
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

/// Check if a property_declaration match contains a readonly modifier.
/// Walks the captures looking for the property_declaration node and checks its children.
fn is_property_readonly(m: &tree_sitter::QueryMatch, _source: &[u8]) -> bool {
    // Find the property_declaration node by looking at the vis modifier's parent
    for cap in m.captures {
        if cap.node.kind() == "visibility_modifier"
            && let Some(parent) = cap.node.parent()
            && parent.kind() == "property_declaration"
        {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.kind() == "readonly_modifier" {
                    return true;
                }
                // Also check for the "readonly" keyword in text
                if child.kind() == "readonly" {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&php_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::from("<?php\nclass BigClass {\n");
        // 10 public + 1 private = 11 methods + 1 class = 12 total, 11/12 > 80%
        for i in 0..10 {
            src.push_str(&format!("  public function method_{}() {{}}\n", i));
        }
        src.push_str("  private function privateMethod() {}\n");
        src.push_str("}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"<?php
class SmallClass {
    public function foo() {}
    private function bar() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"<?php
class UserService {
    public $db;
    public $logger;
    public function findUser() {}
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
    fn no_leaky_for_private_properties() {
        let src = r#"<?php
class UserService {
    private $db;
    protected $logger;
    public function findUser() {}
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
