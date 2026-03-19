use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{body_references_identifier, count_parameters};

use super::primitives::{extract_snippet, has_modifier, node_text};

const IMPORT_THRESHOLD: usize = 15;
const PARAM_THRESHOLD: usize = 5;

pub struct CouplingPipeline;

impl CouplingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_excessive_imports(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Count using_directive nodes at the root level
        let mut count = 0;
        let mut first_using = None;
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "using_directive" {
                count += 1;
                if first_using.is_none() {
                    first_using = Some(child);
                }
            }
        }

        if count > IMPORT_THRESHOLD {
            if let Some(using_node) = first_using {
                let start = using_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "excessive_imports".to_string(),
                    message: format!(
                        "file has {count} using directives (threshold: {IMPORT_THRESHOLD}) \u{2014} consider splitting into smaller files"
                    ),
                    snippet: extract_snippet(source, using_node, 3),
                });
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

    fn check_low_cohesion(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        check_cohesion_recursive(root, source, file_path, self.name(), &mut findings);

        findings
    }
}

fn check_params_recursive(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "method_declaration" {
            if let Some(params) = node.child_by_field_name("parameters") {
                let param_count = count_parameters(params);
                if param_count > PARAM_THRESHOLD {
                    let name = node
                        .child_by_field_name("name")
                        .map(|n| node_text(n, source))
                        .unwrap_or("<anonymous>");

                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "parameter_overload".to_string(),
                        message: format!(
                            "method `{name}` has {param_count} parameters (threshold: {PARAM_THRESHOLD}) \u{2014} consider using a parameter object"
                        ),
                        snippet: extract_snippet(source, node, 1),
                    });
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn check_cohesion_recursive(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        // Look for class_declaration -> declaration_list -> method_declaration
        if node.kind() == "class_declaration" {
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    if child.kind() != "method_declaration" {
                        continue;
                    }

                    // Skip static methods
                    if has_modifier(child, source, "static") {
                        continue;
                    }

                    if let Some(method_body) = child.child_by_field_name("body") {
                        if !body_references_identifier(method_body, source, "this") {
                            let method_name = child
                                .child_by_field_name("name")
                                .map(|n| node_text(n, source))
                                .unwrap_or("<anonymous>");

                            let start = child.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "info".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "low_cohesion".to_string(),
                                message: format!(
                                    "method `{method_name}` does not reference `this` \u{2014} consider making it static"
                                ),
                                snippet: extract_snippet(source, child, 1),
                            });
                        }
                    }
                }
            }
        }

        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            stack.push(child);
        }
    }
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive using directives, parameter overload, and low method cohesion in C#"
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    // ── excessive_imports ──

    #[test]
    fn detects_excessive_imports() {
        let usings: Vec<String> = (0..16)
            .map(|i| format!("using Namespace{i};"))
            .collect();
        let src = format!(
            "{}\n\nclass Foo {{ }}\n",
            usings.join("\n")
        );
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
        let src = r#"
using System;
using System.Linq;

class Foo {
    public void Main() { }
}
"#;
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
        let src = r#"
class Foo {
    public void TooMany(int a, int b, int c, int d, int e, int f) {
        return;
    }
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloaded.len(), 1);
        assert!(overloaded[0].message.contains("TooMany"));
    }

    #[test]
    fn clean_few_parameters() {
        let src = r#"
class Foo {
    public void Ok(int a, int b) {
        return;
    }
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
    private int _x;

    public string DoNothing() {
        return "hello";
    }
}
"#;
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert_eq!(low.len(), 1);
        assert!(low[0].message.contains("DoNothing"));
    }

    #[test]
    fn clean_cohesive_method() {
        let src = r#"
class Svc {
    private int _x;

    public int GetX() {
        return this._x;
    }
}
"#;
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_static_methods() {
        let src = r#"
class Svc {
    private int _x;

    public static string Helper() {
        return "hello";
    }
}
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
