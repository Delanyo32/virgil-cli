use anyhow::Result;
use tree_sitter::Tree;

use super::primitives::{has_modifier, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;

/// Member kinds counted for API surface area analysis inside class bodies.
const MEMBER_KINDS: &[&str] = &[
    "method_declaration",
    "property_declaration",
    "field_declaration",
];

pub struct ApiSurfaceAreaPipeline;

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl Pipeline for ApiSurfaceAreaPipeline {
    fn name(&self) -> &str {
        "api_surface_area"
    }

    fn description(&self) -> &str {
        "Detects excessive public API and leaky abstraction boundaries in C#"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        check_classes_recursive(root, source, file_path, &mut findings);

        findings
    }
}

fn check_classes_recursive(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "class_declaration" {
        let is_public_class =
            has_modifier(node, source, "public") || has_modifier(node, source, "internal");
        let class_name = node
            .child_by_field_name("name")
            .map(|n| node_text(n, source))
            .unwrap_or("<anonymous>");
        let class_line = node.start_position().row as u32 + 1;

        if let Some(body) = node.child_by_field_name("body") {
            let mut total_members = 0usize;
            let mut exported_members = 0usize;
            let mut public_non_readonly_fields: Vec<String> = Vec::new();

            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                let kind = child.kind();
                if !MEMBER_KINDS.contains(&kind) {
                    continue;
                }

                total_members += 1;
                if has_modifier(child, source, "public") || has_modifier(child, source, "internal")
                {
                    exported_members += 1;
                }

                // Leaky abstraction: public field without readonly modifier
                if kind == "field_declaration"
                    && is_public_class
                    && has_modifier(child, source, "public")
                    && !has_modifier(child, source, "readonly")
                {
                    // Extract field name: field_declaration > variable_declaration > variable_declarator > identifier
                    let mut field_cursor = child.walk();
                    for field_child in child.children(&mut field_cursor) {
                        if field_child.kind() == "variable_declaration" {
                            let mut var_cursor = field_child.walk();
                            for var_child in field_child.children(&mut var_cursor) {
                                if var_child.kind() == "variable_declarator" {
                                    let mut decl_cursor = var_child.walk();
                                    for decl_child in var_child.children(&mut decl_cursor) {
                                        if decl_child.kind() == "identifier" {
                                            let field_name = node_text(decl_child, source);
                                            public_non_readonly_fields.push(field_name.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Pattern 1: excessive_public_api
            if total_members >= EXCESSIVE_API_MIN_SYMBOLS {
                let ratio = exported_members as f64 / total_members as f64;
                if ratio > EXCESSIVE_API_EXPORT_RATIO {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: class_line,
                        column: 1,
                        severity: "info".to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "excessive_public_api".to_string(),
                        message: format!(
                            "Class `{}` exports {}/{} members ({:.0}% exported, threshold: >{}%)",
                            class_name,
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
            if is_public_class && !public_non_readonly_fields.is_empty() {
                let field_list = public_non_readonly_fields.join(", ");
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: class_line,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: "api_surface_area".to_string(),
                    pattern: "leaky_abstraction_boundary".to_string(),
                    message: format!(
                        "Public class `{}` has public non-readonly field(s): {} \u{2014} consider using properties with controlled accessors",
                        class_name, field_list
                    ),
                    snippet: String::new(),
                });
            }
        }
    }

    // Recurse into child nodes (handles namespace nesting)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        check_classes_recursive(child, source, file_path, findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn csharp_lang() -> tree_sitter::Language {
        Language::CSharp.tree_sitter_language()
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&csharp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut methods = String::new();
        // 10 public + 1 private = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            methods.push_str(&format!("    public void Method_{}() {{ }}\n", i));
        }
        methods.push_str("    private void PrivateMethod() { }\n");
        let src = format!("public class OrderService {{\n{}}}\n", methods);
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
public class Foo {
    public void A() { }
    public void B() { }
    private void C() { }
    private void D() { }
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"
public class UserRepository {
    public string ConnectionString;
    public int RetryCount;
    public void Save() { }
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
    fn no_leaky_for_readonly_fields() {
        let src = r#"
public class Config {
    public readonly string Name;
    public readonly int MaxRetries;
    public void Load() { }
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
    fn no_leaky_for_private_fields() {
        let src = r#"
public class UserRepository {
    private string _connectionString;
    private int _retryCount;
    public void Save() { }
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
    fn no_leaky_for_non_public_class() {
        let src = r#"
class InternalRepo {
    public string ConnectionString;
    public void Save() { }
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
    fn detects_leaky_in_namespace() {
        let src = r#"
namespace MyApp {
    public class UserRepository {
        public string ConnectionString;
        public void Save() { }
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }
}
