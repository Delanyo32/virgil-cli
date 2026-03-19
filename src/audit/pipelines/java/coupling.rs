use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    body_has_member_access, body_references_identifier, count_nodes_of_kind, count_parameters,
};

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

        let count = count_nodes_of_kind(root, &["import_declaration"]);

        if count > IMPORT_THRESHOLD {
            // Find the first import_declaration for reporting location
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                if child.kind() == "import_declaration" {
                    let start = child.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "excessive_imports".to_string(),
                        message: format!(
                            "file has {count} imports (threshold: {IMPORT_THRESHOLD}) — consider splitting into smaller classes"
                        ),
                        snippet: extract_snippet(source, child, 3),
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

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low method cohesion in Java"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_excessive_imports(tree, source, file_path));
        findings.extend(self.check_parameter_overload(tree, source, file_path));
        findings.extend(self.check_low_cohesion(tree, source, file_path));
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
        let kind = node.kind();
        if kind == "method_declaration" || kind == "constructor_declaration" {
            // Skip constructors — they often need many parameters for initialization
            if kind == "constructor_declaration" {
                // fall through to push children
            } else if let Some(params) = node.child_by_field_name("parameters") {
                let param_count = count_parameters(params);
                if param_count > PARAM_THRESHOLD {
                    let name = node
                        .child_by_field_name("name")
                        .map(|n| node_text(n, source))
                        .unwrap_or("<anonymous>");

                    // Also skip if the method name matches the enclosing class name (constructor pattern)
                    let is_constructor = node
                        .parent()
                        .and_then(|p| {
                            // class_body -> class_declaration
                            p.parent().and_then(|class_node| {
                                class_node
                                    .child_by_field_name("name")
                                    .map(|cn| node_text(cn, source) == name)
                            })
                        })
                        .unwrap_or(false);

                    if !is_constructor {
                        let start = node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "parameter_overload".to_string(),
                            message: format!(
                                "method `{name}` has {param_count} parameters (threshold: {PARAM_THRESHOLD}) — consider using a parameter object"
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

fn check_cohesion_recursive(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        // Look for method_declaration nodes inside class_body
        if node.kind() == "class_body" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "method_declaration" {
                    continue;
                }

                // Skip static methods — they don't need `this`
                if has_modifier(child, source, "static") {
                    continue;
                }

                let body = match child.child_by_field_name("body") {
                    Some(b) => b,
                    None => continue,
                };

                let method_name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source))
                    .unwrap_or("<anonymous>");

                // Check if body references "this" via field_access or identifier
                let uses_this = body_has_member_access(body, source, "field_access", "this")
                    || body_references_identifier(body, source, "this");

                if !uses_this {
                    let start = child.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "low_cohesion".to_string(),
                        message: format!(
                            "method `{method_name}` does not reference `this` — consider making it static"
                        ),
                        snippet: extract_snippet(source, child, 1),
                    });
                }
            }
        }

        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            stack.push(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    // ── excessive_imports ──

    #[test]
    fn detects_excessive_imports() {
        let imports: Vec<String> = (0..16)
            .map(|i| format!("import com.example.pkg{i}.Foo{i};"))
            .collect();
        let src = format!("{}\n\nclass Foo {{}}\n", imports.join("\n"));
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
import java.util.List;
import java.util.Map;

class Foo {
    List<String> items;
    Map<String, String> data;
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
    void tooMany(int a, int b, int c, int d, int e, int f) {
        System.out.println(a + b + c + d + e + f);
    }
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
class Foo {
    int add(int a, int b) {
        return a + b;
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
    private String name;

    String doNothing() {
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
        assert!(low[0].message.contains("doNothing"));
    }

    #[test]
    fn clean_cohesive_method() {
        let src = r#"
class Svc {
    private String name;

    String getName() {
        return this.name;
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
    fn skips_static_method() {
        let src = r#"
class Svc {
    static String utility() {
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
