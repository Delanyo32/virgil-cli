use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    body_references_identifier, count_nodes_of_kind, count_parameters,
};

use super::primitives::{extract_snippet, find_identifier_in_declarator};

const INCLUDE_THRESHOLD: usize = 15;
const PARAM_THRESHOLD: usize = 5;

pub struct CouplingPipeline;

impl CouplingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_excessive_includes(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let count = count_nodes_of_kind(root, &["preproc_include"]);

        if count > INCLUDE_THRESHOLD {
            // Find the first preproc_include for reporting location
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                if child.kind() == "preproc_include" {
                    let start = child.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "excessive_includes".to_string(),
                        message: format!(
                            "file has {count} #include directives (threshold: {INCLUDE_THRESHOLD}) — consider splitting into smaller modules"
                        ),
                        snippet: extract_snippet(source, child, 1),
                    });
                    break;
                }
            }
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

        check_params_recursive(root, source, file_path, self.name(), &mut findings);

        findings
    }

    fn check_low_cohesion(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        check_cohesion_recursive(root, source, file_path, self.name(), &mut findings);

        findings
    }
}

/// Walk all function_definition nodes and check parameter counts.
fn check_params_recursive(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_definition" {
            // In C++ tree-sitter, the parameter_list is inside the function_declarator.
            // function_definition -> declarator: function_declarator -> parameters: parameter_list
            if let Some(declarator) = node.child_by_field_name("declarator") {
                let func_declarator = find_function_declarator(declarator);
                if let Some(func_decl) = func_declarator
                    && let Some(params) = func_decl.child_by_field_name("parameters") {
                        let param_count = count_parameters(params);
                        if param_count > PARAM_THRESHOLD {
                            let name = find_identifier_in_declarator(declarator, source)
                                .unwrap_or_else(|| "<anonymous>".to_string());

                            let start = node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "parameter_overload".to_string(),
                                message: format!(
                                    "function `{name}` has {param_count} parameters (threshold: {PARAM_THRESHOLD}) — consider using a struct or builder"
                                ),
                                snippet: extract_snippet(source, node, 1),
                            });
                        }
                    }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Navigate the declarator chain to find the function_declarator node.
fn find_function_declarator(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    if node.kind() == "function_declarator" {
        return Some(node);
    }
    // The declarator might be wrapped (e.g., pointer_declarator, reference_declarator)
    if let Some(inner) = node.child_by_field_name("declarator") {
        return find_function_declarator(inner);
    }
    // Also check children for qualified_identifier wrapping function_declarator
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|&child| child.kind() == "function_declarator")
}

/// Walk tree to find methods inside class bodies and check if they reference `this`.
/// In C++ tree-sitter, methods inside a class are `function_definition` nodes
/// inside `field_declaration_list` inside `class_specifier`.
fn check_cohesion_recursive(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "class_specifier"
            && let Some(body) = node.child_by_field_name("body") {
                // body is a field_declaration_list
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    if child.kind() == "function_definition" {
                        check_method_cohesion(child, source, file_path, pipeline_name, findings);
                    }
                    // Also check access_specifier blocks — methods may be nested under them
                    // In tree-sitter-cpp, methods after `public:` etc. are still direct children
                    // of field_declaration_list, so we check them directly above.
                }
            }
            // Continue to push children to find nested classes

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Check if a method body references `this`. If not, flag it.
fn check_method_cohesion(
    method_node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if let Some(body) = method_node.child_by_field_name("body")
        && !body_references_identifier(body, source, "this") {
            let name = method_node
                .child_by_field_name("declarator")
                .and_then(|d| find_identifier_in_declarator(d, source))
                .unwrap_or_else(|| "<anonymous>".to_string());

            let start = method_node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "info".to_string(),
                pipeline: pipeline_name.to_string(),
                pattern: "low_cohesion".to_string(),
                message: format!(
                    "method `{name}` does not reference `this` — consider making it a free function or static method"
                ),
                snippet: extract_snippet(source, method_node, 1),
            });
        }
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive includes, parameter overload, and low method cohesion in C++"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_excessive_includes(tree, source, file_path));
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    // ── excessive_includes ──

    #[test]
    fn detects_excessive_includes() {
        let includes: Vec<String> = (0..16).map(|i| format!("#include <header{i}>")).collect();
        let src = format!("{}\n\nint main() {{ return 0; }}\n", includes.join("\n"));
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_includes")
            .collect();
        assert_eq!(excessive.len(), 1);
        assert!(excessive[0].message.contains("16"));
    }

    #[test]
    fn clean_few_includes() {
        let src = r#"
#include <iostream>
#include <vector>

int main() {
    return 0;
}
"#;
        let findings = parse_and_check(src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_includes")
            .collect();
        assert!(excessive.is_empty());
    }

    // ── parameter_overload ──

    #[test]
    fn detects_parameter_overload() {
        let src = r#"
int tooMany(int a, int b, int c, int d, int e, int f) {
    return a + b + c + d + e + f;
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloaded.len(), 1);
        assert!(overloaded[0].message.contains("tooMany"));
    }

    #[test]
    fn clean_few_parameters() {
        let src = r#"
int ok(int a, int b) {
    return a + b;
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloaded.is_empty());
    }

    // ── low_cohesion ──

    #[test]
    fn detects_low_cohesion() {
        let src = r#"
class Svc {
public:
    int doNothing() {
        return 42;
    }
};
"#;
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert_eq!(low.len(), 1);
        assert!(low[0].message.contains("doNothing"));
    }

    #[test]
    fn clean_cohesive_method() {
        let src = r#"
class Svc {
    int value;
public:
    int getValue() {
        return this->value;
    }
};
"#;
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
