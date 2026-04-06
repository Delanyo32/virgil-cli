use anyhow::Result;
use tree_sitter::Tree;

use super::primitives::{
    has_csharp_attribute, has_modifier, is_csharp_suppressed, is_generated_code, node_text,
};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::is_test_file;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;
const EXCESSIVE_API_WARNING_RATIO: f64 = 0.90;
const LEAKY_FIELD_ERROR_COUNT: usize = 5;

/// Member kinds counted for API surface area analysis inside class bodies.
const MEMBER_KINDS: &[&str] = &[
    "method_declaration",
    "property_declaration",
    "field_declaration",
    "event_field_declaration",
    "indexer_declaration",
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
        if is_test_file(file_path) || is_generated_code(file_path, source) {
            return vec![];
        }
        let mut findings = Vec::new();
        let root = tree.root_node();

        check_classes_recursive(root, source, file_path, &mut findings);

        findings
    }
}

/// Returns true if this class is an ASP.NET controller.
/// Heuristic 1: class name ends with "Controller".
/// Heuristic 2: class has [ApiController] or [Controller] attribute.
fn is_aspnet_controller(node: tree_sitter::Node, source: &[u8], class_name: &str) -> bool {
    if class_name.ends_with("Controller") {
        return true;
    }
    has_csharp_attribute(node, source, "ApiController")
        || has_csharp_attribute(node, source, "Controller")
}

fn check_classes_recursive(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "class_declaration"
        && !is_csharp_suppressed(source, node, "api_surface_area")
    {
        let is_public_class = has_modifier(node, source, "public");
        let class_name = node
            .child_by_field_name("name")
            .map(|n| node_text(n, source))
            .unwrap_or("<anonymous>");
        let class_line = node.start_position().row as u32 + 1;
        let skip_api_ratio = is_aspnet_controller(node, source, class_name);

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
                if has_modifier(child, source, "public") {
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
            if !skip_api_ratio && total_members >= EXCESSIVE_API_MIN_SYMBOLS {
                let ratio = exported_members as f64 / total_members as f64;
                if ratio > EXCESSIVE_API_EXPORT_RATIO {
                    let severity = if ratio > EXCESSIVE_API_WARNING_RATIO {
                        "warning"
                    } else {
                        "info"
                    };
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: class_line,
                        column: 1,
                        severity: severity.to_string(),
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
                let leaky_severity = if public_non_readonly_fields.len() > LEAKY_FIELD_ERROR_COUNT {
                    "error"
                } else {
                    "warning"
                };
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: class_line,
                    column: 1,
                    severity: leaky_severity.to_string(),
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
        pipeline.check(&tree, source.as_bytes(), "MyApp.cs")
    }

    fn parse_and_check_file(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&csharp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
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
    fn test_excessive_api_severity_graduation() {
        // 10/10 public (100%) => "warning"
        let mut all_public = String::new();
        for i in 0..10 {
            all_public.push_str(&format!("    public void Method_{}() {{ }}\n", i));
        }
        let src_warn = format!("public class OrderService {{\n{}}}\n", all_public);
        let warn_findings = parse_and_check(&src_warn);
        let finding = warn_findings
            .iter()
            .find(|f| f.pattern == "excessive_public_api")
            .expect("10/10 public must trigger excessive_public_api");
        assert_eq!(finding.severity, "warning", "100% exported must be severity 'warning'");

        // 9/11 (81.8%) => "info"
        let src_info = r#"
public class OrderService {
    public void A() { }
    public void B() { }
    public void C() { }
    public void D() { }
    public void E() { }
    public void F() { }
    public void G() { }
    public void H() { }
    public void I() { }
    private void P1() { }
    private void P2() { }
}
"#;
        let info_findings = parse_and_check(src_info);
        let info_finding = info_findings
            .iter()
            .find(|f| f.pattern == "excessive_public_api")
            .expect("9/11 (81.8%) must trigger excessive_public_api");
        assert_eq!(info_finding.severity, "info", "81.8% exported must be severity 'info'");
    }

    #[test]
    fn test_leaky_abstraction_severity_graduation() {
        // 6 public fields => "error"
        let src_error = r#"
public class OrderRepository {
    public string Field1;
    public string Field2;
    public string Field3;
    public string Field4;
    public string Field5;
    public string Field6;
}
"#;
        let error_findings = parse_and_check(src_error);
        let finding = error_findings
            .iter()
            .find(|f| f.pattern == "leaky_abstraction_boundary")
            .expect("6 public fields must trigger leaky_abstraction_boundary");
        assert_eq!(finding.severity, "error", "6 public mutable fields must be severity 'error'");

        // 2 public fields => "warning"
        let src_warn = r#"
public class OrderRepository {
    public string ConnectionString;
    public int RetryCount;
    public void Save() { }
}
"#;
        let warn_findings = parse_and_check(src_warn);
        let warn_finding = warn_findings
            .iter()
            .find(|f| f.pattern == "leaky_abstraction_boundary")
            .expect("2 public fields must trigger leaky_abstraction_boundary");
        assert_eq!(warn_finding.severity, "warning", "2 public mutable fields must be severity 'warning'");
    }

    #[test]
    fn test_controller_not_excessive_api() {
        let src = r#"
[ApiController]
public class CustomerController {
    public void GetAll() { }
    public void GetById() { }
    public void Create() { }
    public void Update() { }
    public void Delete() { }
    public void Search() { }
    public void Export() { }
    public void Import() { }
    public void Validate() { }
    public void BatchUpdate() { }
    private void InternalHelper() { }
}
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings.iter().any(|f| f.pattern == "excessive_public_api"),
            "ASP.NET controllers idiomatically expose all action methods as public — must not flag"
        );
    }

    #[test]
    fn test_event_members_counted_in_ratio() {
        // 5 public methods + 5 public events + 1 private method = 11 total, 10 exported = 90.9% > 80%
        let src = r#"
public class OrderService {
    public void Create() { }
    public void Read() { }
    public void Update() { }
    public void Delete() { }
    public void List() { }
    public event EventHandler Created;
    public event EventHandler Updated;
    public event EventHandler Deleted;
    public event EventHandler Listed;
    public event EventHandler Exported;
    private void InternalHelper() { }
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings.iter().any(|f| f.pattern == "excessive_public_api"),
            "event declarations must be counted as members for the API ratio"
        );
    }

    #[test]
    fn test_file_not_analyzed() {
        let mut methods = String::new();
        for i in 0..10 {
            methods.push_str(&format!("    public void Method_{}() {{ }}\n", i));
        }
        methods.push_str("    private void PrivateMethod() { }\n");
        let src = format!("public class OrderServiceTests {{\n{}}}\n", methods);
        let findings = parse_and_check_file(&src, "OrderServiceTests.cs");
        assert!(findings.is_empty(), "test files must not produce any findings");
    }

    #[test]
    fn test_generated_file_not_analyzed() {
        let src = r#"
public class Form1 {
    public string ConnectionString;
    public void Save() { }
}
"#;
        let findings = parse_and_check_file(src, "Form1.Designer.cs");
        assert!(findings.is_empty(), "Designer.cs files must not produce any findings");
    }

    #[test]
    fn test_internal_class_not_excessive_api() {
        let mut methods = String::new();
        for i in 0..10 {
            methods.push_str(&format!("    internal void Method_{}() {{ }}\n", i));
        }
        let src = format!("public class InternalHelper {{\n{}}}\n", methods);
        let findings = parse_and_check(&src);
        assert!(
            !findings.iter().any(|f| f.pattern == "excessive_public_api"),
            "internal methods are not part of the public API and must not count as exported"
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
