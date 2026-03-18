use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_top_level_definitions;
use crate::language::Language;
use super::primitives::{find_capture_index, node_text};

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;
const LEAKY_ABSTRACTION_MIN_PUBLIC_ATTRS: usize = 3;

const PYTHON_SYMBOL_KINDS: &[&str] = &[
    "function_definition",
    "class_definition",
    "decorated_definition",
];

fn python_lang() -> tree_sitter::Language {
    Language::Python.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    symbol_query: Arc<Query>,
    class_init_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        // Match top-level function/class definitions with their names
        let symbol_query_str = r#"
[
  (function_definition
    name: (identifier) @name) @sym
  (class_definition
    name: (identifier) @name) @sym
  (decorated_definition
    definition: (function_definition
      name: (identifier) @name)) @sym
  (decorated_definition
    definition: (class_definition
      name: (identifier) @name)) @sym
]
"#;
        let symbol_query = Query::new(&python_lang(), symbol_query_str)
            .with_context(|| "failed to compile symbol query for Python API surface")?;

        // Match class definitions with their names for leaky abstraction detection.
        // We'll manually walk the class body to find __init__ and its self.attr assignments.
        let class_init_query_str = r#"
(class_definition
  name: (identifier) @class_name
  body: (block) @class_body) @class_def
"#;
        let class_init_query = Query::new(&python_lang(), class_init_query_str)
            .with_context(|| "failed to compile class init query for Python API surface")?;

        Ok(Self {
            symbol_query: Arc::new(symbol_query),
            class_init_query: Arc::new(class_init_query),
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
        let total_symbols = count_top_level_definitions(root, PYTHON_SYMBOL_KINDS);

        let mut exported_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let sym_idx = find_capture_index(&self.symbol_query, "sym");
            let name_idx = find_capture_index(&self.symbol_query, "name");
            let mut matches = cursor.matches(&self.symbol_query, root, source);
            while let Some(m) = matches.next() {
                let mut is_top_level = false;
                let mut is_exported = false;

                for cap in m.captures {
                    if cap.index as usize == sym_idx {
                        is_top_level = cap.node.parent().map_or(false, |p| p.kind() == "module");
                    }
                    if cap.index as usize == name_idx {
                        let name = node_text(cap.node, source);
                        is_exported = !name.starts_with('_');
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
        // Find exported classes with public (non-underscore) self.attr assignments in __init__
        {
            let mut cursor = QueryCursor::new();
            let class_name_idx = find_capture_index(&self.class_init_query, "class_name");
            let class_body_idx = find_capture_index(&self.class_init_query, "class_body");
            let mut matches = cursor.matches(&self.class_init_query, root, source);
            let mut reported_classes = std::collections::HashSet::new();

            while let Some(m) = matches.next() {
                let mut class_name = "";
                let mut class_line = 0u32;
                let mut class_body_node = None;

                for cap in m.captures {
                    if cap.index as usize == class_name_idx {
                        class_name = node_text(cap.node, source);
                        class_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == class_body_idx {
                        class_body_node = Some(cap.node);
                    }
                }

                // Skip private classes (name starts with _)
                if class_name.starts_with('_') || class_name.is_empty() {
                    continue;
                }

                if let Some(body) = class_body_node {
                    // Find __init__ method in class body
                    let public_attrs = count_public_init_attrs(body, source);

                    if public_attrs >= LEAKY_ABSTRACTION_MIN_PUBLIC_ATTRS
                        && !reported_classes.contains(class_name)
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
                                "Public class `{}` has {} public instance attributes — consider prefixing internals with `_`",
                                class_name, public_attrs
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

/// Count public (non-underscore) self.attribute assignments in __init__ methods
/// within a class body node.
fn count_public_init_attrs(class_body: tree_sitter::Node, source: &[u8]) -> usize {
    let mut count = 0;
    let mut cursor = class_body.walk();

    for child in class_body.children(&mut cursor) {
        // Look for function_definition or decorated_definition containing __init__
        let func_node = if child.kind() == "function_definition" {
            Some(child)
        } else if child.kind() == "decorated_definition" {
            child.child_by_field_name("definition")
                .filter(|d| d.kind() == "function_definition")
        } else {
            None
        };

        if let Some(func) = func_node {
            // Check if this is __init__
            if let Some(name_node) = func.child_by_field_name("name") {
                let name = name_node.utf8_text(source).unwrap_or("");
                if name == "__init__" {
                    if let Some(body) = func.child_by_field_name("body") {
                        count += count_public_self_attrs_in_body(body, source);
                    }
                }
            }
        }
    }

    count
}

/// Walk the body of __init__ and count `self.attr = ...` where attr doesn't start with `_`.
fn count_public_self_attrs_in_body(body: tree_sitter::Node, source: &[u8]) -> usize {
    let mut count = 0;
    let mut seen_attrs = std::collections::HashSet::new();
    count_self_attrs_recursive(body, source, &mut count, &mut seen_attrs);
    count
}

fn count_self_attrs_recursive(
    node: tree_sitter::Node,
    source: &[u8],
    count: &mut usize,
    seen: &mut std::collections::HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            // Look for assignment: self.attr = ...
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "assignment" {
                    if let Some(left) = inner.child_by_field_name("left") {
                        if left.kind() == "attribute" {
                            // Check object is "self"
                            if let Some(obj) = left.child_by_field_name("object") {
                                if obj.utf8_text(source).unwrap_or("") == "self" {
                                    if let Some(attr) = left.child_by_field_name("attribute") {
                                        let attr_name = attr.utf8_text(source).unwrap_or("");
                                        if !attr_name.starts_with('_') && !attr_name.is_empty() {
                                            if seen.insert(attr_name.to_string()) {
                                                *count += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Recurse into if/else blocks within __init__ that may also set attributes
        count_self_attrs_recursive(child, source, count, seen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&python_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::new();
        // 10 public + 1 private = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("def func_{}():\n    pass\n", i));
        }
        src.push_str("def _private_func():\n    pass\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
def foo():
    pass
def bar():
    pass
def _baz():
    pass
def _qux():
    pass
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"
class ConnectionPool:
    def __init__(self, dsn, max_size=10):
        self.connections = []
        self.available = []
        self.dsn = dsn
        self.max_size = max_size

    def acquire(self):
        pass
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }

    #[test]
    fn no_leaky_for_private_attrs() {
        let src = r#"
class ConnectionPool:
    def __init__(self, dsn, max_size=10):
        self._connections = []
        self._available = []
        self._dsn = dsn
        self._max_size = max_size

    def acquire(self):
        pass
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }

    #[test]
    fn no_leaky_for_private_class() {
        let src = r#"
class _InternalPool:
    def __init__(self):
        self.connections = []
        self.available = []
        self.dsn = ""

    def acquire(self):
        pass
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }

    #[test]
    fn no_leaky_for_few_public_attrs() {
        let src = r#"
class SmallClass:
    def __init__(self):
        self.name = ""
        self.value = 0
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }
}
