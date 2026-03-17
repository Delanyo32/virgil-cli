use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{body_has_member_access, count_nodes_of_kind};

use super::primitives::{extract_snippet, node_text};

const EXCESSIVE_IMPORTS_THRESHOLD: usize = 15;
const PARAMETER_OVERLOAD_THRESHOLD: usize = 5;

pub struct CouplingPipeline;

impl CouplingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_excessive_imports(
        &self,
        tree: &Tree,
        _source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let import_count = count_nodes_of_kind(
            root,
            &["import_statement", "import_from_statement"],
        );

        if import_count > EXCESSIVE_IMPORTS_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "excessive_imports".to_string(),
                message: format!(
                    "file has {import_count} imports (threshold: {EXCESSIVE_IMPORTS_THRESHOLD}) \
                     — consider splitting into smaller modules"
                ),
                snippet: String::new(),
            });
        }

        findings
    }

    fn check_parameter_overload(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        collect_parameter_overloads(
            root,
            source,
            file_path,
            self.name(),
            &mut findings,
        );

        findings
    }

    fn check_low_cohesion(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        collect_low_cohesion_methods(
            root,
            source,
            file_path,
            self.name(),
            &mut findings,
        );

        findings
    }
}

/// Count parameters in a Python parameters node, excluding `self` and `cls`.
fn count_python_parameters(params_node: tree_sitter::Node, source: &[u8]) -> usize {
    let mut count = 0;
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        let kind = child.kind();
        // Skip variadic parameters (*args, **kwargs)
        if kind == "list_splat_pattern" || kind == "dictionary_splat_pattern" {
            continue;
        }
        // For identifiers (untyped params), check if it's self/cls
        if kind == "identifier" {
            let name = node_text(child, source);
            if name == "self" || name == "cls" {
                continue;
            }
        }
        // For typed_parameter, check the name
        if kind == "typed_parameter" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, source);
                if name == "self" || name == "cls" {
                    continue;
                }
            }
        }
        // For default_parameter, check the name
        if kind == "default_parameter" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, source);
                if name == "self" || name == "cls" {
                    continue;
                }
            }
        }
        // For typed_default_parameter, check the name
        if kind == "typed_default_parameter" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, source);
                if name == "self" || name == "cls" {
                    continue;
                }
            }
        }
        count += 1;
    }
    count
}

/// Check if a function has a `self` parameter.
fn has_self_parameter(params_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if node_text(child, source) == "self" {
                    return true;
                }
            }
            "typed_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if node_text(name_node, source) == "self" {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Walk tree looking for function_definition nodes and check parameter counts.
fn collect_parameter_overloads(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "function_definition" {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Some(params_node) = node.child_by_field_name("parameters") {
                let param_count = count_python_parameters(params_node, source);
                if param_count > PARAMETER_OVERLOAD_THRESHOLD {
                    let fn_name = node_text(name_node, source);
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "parameter_overload".to_string(),
                        message: format!(
                            "function `{fn_name}` has {param_count} parameters \
                             (threshold: {PARAMETER_OVERLOAD_THRESHOLD}) — consider using \
                             a configuration object or dataclass"
                        ),
                        snippet: extract_snippet(source, node, 1),
                    });
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_parameter_overloads(child, source, file_path, pipeline_name, findings);
    }
}

/// Walk tree looking for methods inside class_definition that have a `self` parameter
/// but don't access `self` in their body.
fn collect_low_cohesion_methods(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "class_definition" {
        // Walk the class body looking for methods
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                let func_node = match child.kind() {
                    "function_definition" => child,
                    "decorated_definition" => {
                        match find_inner_function(child) {
                            Some(f) => f,
                            None => continue,
                        }
                    }
                    _ => continue,
                };

                let name_node = match func_node.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                let fn_name = node_text(name_node, source);

                // Skip dunder methods
                if fn_name.starts_with("__") && fn_name.ends_with("__") {
                    continue;
                }

                let params_node = match func_node.child_by_field_name("parameters") {
                    Some(p) => p,
                    None => continue,
                };

                // Only check methods that have a `self` parameter
                if !has_self_parameter(params_node, source) {
                    continue;
                }

                let func_body = match func_node.child_by_field_name("body") {
                    Some(b) => b,
                    None => continue,
                };

                // Skip trivial bodies (just `pass` or `...`)
                if is_trivial_body(func_body, source) {
                    continue;
                }

                // Check if the body accesses self via attribute access
                if !body_has_member_access(func_body, source, "attribute", "self") {
                    let start = func_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "low_cohesion".to_string(),
                        message: format!(
                            "method `{fn_name}` has a `self` parameter but never accesses \
                             instance attributes — consider making it a function or @staticmethod"
                        ),
                        snippet: extract_snippet(source, func_node, 1),
                    });
                }
            }
        }

        // Don't recurse into nested classes from here; we'll catch them naturally
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_low_cohesion_methods(child, source, file_path, pipeline_name, findings);
    }
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

/// Check if a function body is trivial (just `pass` or `...`).
fn is_trivial_body(body: tree_sitter::Node, _source: &[u8]) -> bool {
    if body.named_child_count() != 1 {
        return false;
    }
    if let Some(child) = body.named_child(0) {
        let kind = child.kind();
        if kind == "pass_statement" {
            return true;
        }
        if kind == "expression_statement" {
            if let Some(expr) = child.named_child(0) {
                if expr.kind() == "ellipsis" {
                    return true;
                }
            }
        }
    }
    false
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low cohesion in Python"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_excessive_imports(tree, source, file_path));
        findings.extend(self.check_parameter_overload(tree, source, file_path));
        findings.extend(self.check_low_cohesion(tree, source, file_path));
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
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    // ── excessive_imports ──

    #[test]
    fn detects_excessive_imports() {
        let imports: String = (0..16)
            .map(|i| format!("import mod{i}\n"))
            .collect();
        let src = format!("{imports}\ndef main():\n    pass\n");
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert_eq!(excessive.len(), 1);
        assert!(excessive[0].message.contains("16"));
    }

    #[test]
    fn clean_few_imports() {
        let src = "\
import os
import sys

def main():
    pass
";
        let findings = parse_and_check(src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert!(excessive.is_empty());
    }

    // ── parameter_overload ──

    #[test]
    fn detects_parameter_overload() {
        let src = "\
def process(a, b, c, d, e, f):
    pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloads.len(), 1);
        assert!(overloads[0].message.contains("process"));
    }

    #[test]
    fn clean_few_parameters() {
        let src = "\
def process(a, b, c):
    pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloads.is_empty());
    }

    #[test]
    fn excludes_self_from_count() {
        let src = "\
class Foo:
    def process(self, a, b, c, d, e):
        pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloads.is_empty());
    }

    #[test]
    fn detects_overload_even_with_self() {
        let src = "\
class Foo:
    def process(self, a, b, c, d, e, f):
        pass
";
        let findings = parse_and_check(src);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloads.len(), 1);
    }

    // ── low_cohesion ──

    #[test]
    fn detects_low_cohesion() {
        let src = "\
class Calculator:
    def add(self, a, b):
        return a + b
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert_eq!(low.len(), 1);
        assert!(low[0].message.contains("add"));
    }

    #[test]
    fn clean_uses_self() {
        let src = "\
class Calculator:
    def add(self, a, b):
        self.result = a + b
        return self.result
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_trivial_methods() {
        let src = "\
class Base:
    def do_nothing(self):
        pass
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_static_methods() {
        // No self parameter, so not flagged
        let src = "\
class Util:
    def helper(a, b):
        return a + b
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_dunder_methods() {
        let src = "\
class Foo:
    def __repr__(self):
        return 'Foo()'
";
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    // ── metadata ──

    #[test]
    fn metadata_check() {
        let pipeline = CouplingPipeline::new().unwrap();
        assert_eq!(pipeline.name(), "coupling");
        assert!(!pipeline.description().is_empty());
    }
}
