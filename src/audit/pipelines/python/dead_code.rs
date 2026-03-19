use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_all_identifier_occurrences, find_unreachable_after};

use super::primitives::{extract_snippet, node_text};

pub struct DeadCodePipeline;

impl DeadCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_unused_private_functions(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Build identifier count map once for the entire file — O(n) instead of O(n*m).
        let id_counts = count_all_identifier_occurrences(root, source);

        // Walk root children looking for function_definition with names starting with _
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            let func_node = match child.kind() {
                "function_definition" => child,
                "decorated_definition" => {
                    // Unwrap to inner function_definition
                    match find_inner_function(child) {
                        Some(f) => f,
                        None => continue,
                    }
                }
                _ => continue,
            };

            // Only module-level functions (parent is module)
            let name_node = match func_node.child_by_field_name("name") {
                Some(n) => n,
                None => continue,
            };

            let name = node_text(name_node, source);
            if name.is_empty() || !name.starts_with('_') {
                continue;
            }

            // Skip dunder methods
            if name.starts_with("__") && name.ends_with("__") {
                continue;
            }

            // Skip functions listed in __all__
            if is_in_all_list(root, source, name) {
                continue;
            }

            // Skip decorated functions (may be registered callbacks)
            if has_decorator(child) {
                continue;
            }

            // The declaration itself counts as 1. If total <= 1, the function is unused.
            let total_count = id_counts.get(name).copied().unwrap_or(0);
            if total_count <= 1 {
                let start = child.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_private_function".to_string(),
                    message: format!("private function `{name}` appears unused within this file"),
                    snippet: extract_snippet(source, child, 1),
                });
            }
        }

        findings
    }

    fn check_unused_imports(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Collect identifiers from non-import nodes
        let mut non_import_ids: HashSet<String> = HashSet::new();
        let mut root_cursor = root.walk();
        for child in root.children(&mut root_cursor) {
            if child.kind() != "import_statement" && child.kind() != "import_from_statement" {
                collect_identifiers_into(child, source, &mut non_import_ids);
            }
        }

        // Walk for import_statement and import_from_statement
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            match child.kind() {
                "import_statement" => {
                    // `import foo` or `import foo.bar`
                    // The imported name used in code is the first segment (or alias)
                    let mut inner_cursor = child.walk();
                    for import_child in child.named_children(&mut inner_cursor) {
                        match import_child.kind() {
                            "dotted_name" => {
                                // `import foo.bar` => user references `foo`
                                let first_seg = import_child.named_child(0);
                                if let Some(seg) = first_seg {
                                    let name = node_text(seg, source);
                                    if !name.is_empty() && !non_import_ids.contains(name) {
                                        let start = child.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "info".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "unused_import".to_string(),
                                            message: format!("import `{name}` appears unused"),
                                            snippet: extract_snippet(source, child, 1),
                                        });
                                    }
                                }
                            }
                            "aliased_import" => {
                                // `import foo as bar` => user references `bar`
                                if let Some(alias) = import_child.child_by_field_name("alias") {
                                    let name = node_text(alias, source);
                                    if !name.is_empty() && !non_import_ids.contains(name) {
                                        let start = child.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "info".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "unused_import".to_string(),
                                            message: format!("import `{name}` appears unused"),
                                            snippet: extract_snippet(source, child, 1),
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "import_from_statement" => {
                    // `from foo import bar, baz` or `from foo import bar as qux`
                    let module_name_node = child.child_by_field_name("module_name");
                    let mut inner_cursor = child.walk();
                    for import_child in child.named_children(&mut inner_cursor) {
                        // Skip the module name node
                        if let Some(mn) = module_name_node {
                            if import_child.id() == mn.id() {
                                continue;
                            }
                        }
                        match import_child.kind() {
                            "dotted_name" => {
                                // This is an imported name like `from os import path`
                                // Extract the last segment
                                let text = node_text(import_child, source);
                                let name = text.rsplit('.').next().unwrap_or(text);
                                if !name.is_empty() && name != "*" && !non_import_ids.contains(name)
                                {
                                    let start = import_child.start_position();
                                    findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "info".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "unused_import".to_string(),
                                        message: format!("import `{name}` appears unused"),
                                        snippet: extract_snippet(source, child, 1),
                                    });
                                }
                            }
                            "aliased_import" => {
                                // `from foo import bar as qux` => user references `qux`
                                if let Some(alias) = import_child.child_by_field_name("alias") {
                                    let name = node_text(alias, source);
                                    if !name.is_empty() && !non_import_ids.contains(name) {
                                        let start = import_child.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "info".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "unused_import".to_string(),
                                            message: format!("import `{name}` appears unused"),
                                            snippet: extract_snippet(source, child, 1),
                                        });
                                    }
                                }
                            }
                            "identifier" => {
                                // Direct name import like `from foo import bar`
                                // But skip the module_name field
                                let is_module_name = child
                                    .child_by_field_name("module_name")
                                    .map(|n| n.id() == import_child.id())
                                    .unwrap_or(false);
                                if is_module_name {
                                    continue;
                                }
                                let name = node_text(import_child, source);
                                if !name.is_empty() && name != "*" && !non_import_ids.contains(name)
                                {
                                    let start = import_child.start_position();
                                    findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "info".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "unused_import".to_string(),
                                        message: format!("import `{name}` appears unused"),
                                        snippet: extract_snippet(source, child, 1),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        findings
    }

    fn check_unreachable_code(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let return_kinds = &[
            "return_statement",
            "break_statement",
            "continue_statement",
            "raise_statement",
        ];
        collect_unreachable_in_blocks(
            root,
            source,
            return_kinds,
            file_path,
            self.name(),
            &mut findings,
        );

        findings
    }
}

/// Check if a function name appears in an `__all__` list at the module level.
fn is_in_all_list(root: tree_sitter::Node, source: &[u8], name: &str) -> bool {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            // Look for assignment: __all__ = [...]
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "assignment" {
                    if let Some(lhs) = inner.child_by_field_name("left") {
                        if lhs.kind() == "identifier" {
                            let lhs_text = node_text(lhs, source);
                            if lhs_text == "__all__" {
                                // Check if the function name appears in the right side text
                                if let Some(rhs) = inner.child_by_field_name("right") {
                                    let rhs_text = rhs.utf8_text(source).unwrap_or("");
                                    // Check for the name as a string literal in the list
                                    if rhs_text.contains(&format!("\"{}\"", name))
                                        || rhs_text.contains(&format!("'{}'", name))
                                    {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

/// Check if a node has a decorator (either it's a decorated_definition, or
/// it's a function_definition whose parent is a decorated_definition).
fn has_decorator(node: tree_sitter::Node) -> bool {
    // If this is already a decorated_definition, it has decorators
    if node.kind() == "decorated_definition" {
        return true;
    }
    // If it's a function_definition whose parent is decorated_definition
    if node.kind() == "function_definition" {
        if let Some(parent) = node.parent() {
            if parent.kind() == "decorated_definition" {
                return true;
            }
        }
    }
    false
}

/// Find the inner function_definition inside a decorated_definition.
fn find_inner_function(decorated: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut cursor = decorated.walk();
    for child in decorated.children(&mut cursor) {
        if child.kind() == "function_definition" {
            return Some(child);
        }
    }
    None
}

fn collect_identifiers_into(root: tree_sitter::Node, source: &[u8], ids: &mut HashSet<String>) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "identifier" {
            if let Ok(text) = node.utf8_text(source) {
                ids.insert(text.to_string());
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn collect_unreachable_in_blocks(
    root: tree_sitter::Node,
    _source: &[u8],
    return_kinds: &[&str],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "block" {
            let unreachable = find_unreachable_after(node, return_kinds);
            for (line, col) in unreachable {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line,
                    column: col,
                    severity: "warning".to_string(),
                    pipeline: pipeline_name.to_string(),
                    pattern: "unreachable_code".to_string(),
                    message: "code is unreachable after return/break/continue/raise".to_string(),
                    snippet: String::new(),
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused private functions, unused imports, and unreachable code in Python"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_unused_private_functions(tree, source, file_path));
        findings.extend(self.check_unused_imports(tree, source, file_path));
        findings.extend(self.check_unreachable_code(tree, source, file_path));
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    // ── unused_private_function ──

    #[test]
    fn detects_unused_private_function() {
        let src = "\
def main():
    pass

def _unused_helper():
    pass
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("_unused_helper"));
    }

    #[test]
    fn clean_used_private_function() {
        let src = "\
def _helper():
    return 42

def main():
    x = _helper()
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn skips_public_function() {
        let src = "\
def public_but_unused():
    pass
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn skips_dunder_methods() {
        let src = "\
def __init__():
    pass
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(unused.is_empty());
    }

    // ── unused_import ──

    #[test]
    fn detects_unused_import() {
        let src = "\
import os

def main():
    x = 1
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("os"));
    }

    #[test]
    fn clean_used_import() {
        let src = "\
import os

def main():
    os.path.exists('.')
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn detects_unused_from_import() {
        let src = "\
from os import path

def main():
    x = 1
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("path"));
    }

    #[test]
    fn clean_used_from_import() {
        let src = "\
from os import path

def main():
    path.exists('.')
";
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert!(unused.is_empty());
    }

    // ── unreachable_code ──

    #[test]
    fn detects_unreachable_after_return() {
        let src = "\
def foo():
    return 1
    x = 2
";
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(!unreachable.is_empty());
    }

    #[test]
    fn detects_unreachable_after_raise() {
        let src = "\
def foo():
    raise ValueError('bad')
    x = 2
";
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(!unreachable.is_empty());
    }

    #[test]
    fn clean_no_unreachable() {
        let src = "\
def foo():
    x = 1
    return x
";
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(unreachable.is_empty());
    }

    // ── metadata ──

    #[test]
    fn metadata_check() {
        let pipeline = DeadCodePipeline::new().unwrap();
        assert_eq!(pipeline.name(), "dead_code");
        assert!(!pipeline.description().is_empty());
    }
}
