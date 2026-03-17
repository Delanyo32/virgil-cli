use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    body_references_identifier, count_nodes_of_kind, count_parameters,
};
use super::primitives::extract_snippet;

const EXCESSIVE_IMPORT_THRESHOLD: usize = 15;
const PARAMETER_OVERLOAD_THRESHOLD: usize = 5;

pub struct CouplingPipeline;

impl CouplingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low-cohesion methods in Rust"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── excessive_imports ──────────────────────────────────────────
        let use_count = count_nodes_of_kind(root, &["use_declaration"]);
        if use_count > EXCESSIVE_IMPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "excessive_imports".to_string(),
                message: format!(
                    "file has {use_count} use declarations (threshold: {EXCESSIVE_IMPORT_THRESHOLD}) — consider splitting this module or re-exporting via a prelude"
                ),
                snippet: String::new(),
            });
        }

        // ── parameter_overload ─────────────────────────────────────────
        check_parameter_overload(root, source, file_path, self.name(), &mut findings);

        // ── low_cohesion ───────────────────────────────────────────────
        check_low_cohesion(root, source, file_path, self.name(), &mut findings);

        findings
    }
}

/// Find function_item nodes with too many parameters.
fn check_parameter_overload(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "function_item" {
        if let Some(params) = node.child_by_field_name("parameters") {
            let param_count = count_parameters(params);
            if param_count > PARAMETER_OVERLOAD_THRESHOLD {
                let fn_name = node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
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
                        "function `{fn_name}` has {param_count} parameters (threshold: {PARAMETER_OVERLOAD_THRESHOLD}) — consider grouping into a struct or builder"
                    ),
                    snippet: extract_snippet(source, node, 3),
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        check_parameter_overload(child, source, file_path, pipeline_name, findings);
    }
}

/// Find methods inside impl blocks that take `self` but never reference it in the body.
fn check_low_cohesion(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "impl_item" {
        if let Some(decl_list) = node.child_by_field_name("body") {
            let mut cursor = decl_list.walk();
            for child in decl_list.named_children(&mut cursor) {
                if child.kind() == "function_item" {
                    check_method_cohesion(child, source, file_path, pipeline_name, findings);
                }
            }
        }
        // Don't recurse into nested items from here; we handle them below.
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        check_low_cohesion(child, source, file_path, pipeline_name, findings);
    }
}

/// Check a single method: if it has a self_parameter but the body never references `self`.
fn check_method_cohesion(
    func_node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let params = match func_node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return,
    };

    // Check if the function has a self_parameter
    let has_self_param = {
        let mut cursor = params.walk();
        let mut found = false;
        for child in params.named_children(&mut cursor) {
            if child.kind() == "self_parameter" {
                found = true;
                break;
            }
        }
        found
    };

    if !has_self_param {
        return;
    }

    let body = match func_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    // Check if body references `self` (the identifier, not the parameter declaration)
    if !body_references_identifier(body, source, "self") {
        let fn_name = func_node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .unwrap_or("<anonymous>");
        let start = func_node.start_position();
        findings.push(AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "info".to_string(),
            pipeline: pipeline_name.to_string(),
            pattern: "low_cohesion".to_string(),
            message: format!(
                "method `{fn_name}` takes self but never references it — consider making it a free function or associated function"
            ),
            snippet: extract_snippet(source, func_node, 3),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_excessive_imports() {
        let imports: Vec<String> = (0..20)
            .map(|i| format!("use crate::module_{i}::Thing{i};"))
            .collect();
        let src = format!("{}\n\nfn main() {{}}", imports.join("\n"));
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert!(
            !excessive.is_empty(),
            "should flag excessive imports"
        );
        assert!(excessive[0].message.contains("20"));
    }

    #[test]
    fn no_excessive_imports_when_few() {
        let src = r#"
use std::fmt;
use std::io;

fn main() {}
"#;
        let findings = parse_and_check(src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert!(excessive.is_empty(), "few imports should not be flagged");
    }

    #[test]
    fn detects_parameter_overload() {
        let src = r#"
fn too_many(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32) -> i32 {
    a + b + c + d + e + f
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(
            !overloaded.is_empty(),
            "should flag function with 6 parameters"
        );
        assert!(overloaded[0].message.contains("too_many"));
    }

    #[test]
    fn no_parameter_overload_when_few() {
        let src = r#"
fn fine(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloaded.is_empty(), "2 parameters should not be flagged");
    }

    #[test]
    fn self_parameter_not_counted() {
        let src = r#"
struct Foo;
impl Foo {
    fn method(&self, a: i32, b: i32, c: i32, d: i32, e: i32) -> i32 {
        self.bar() + a + b + c + d + e
    }

    fn bar(&self) -> i32 { 0 }
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        // 5 params (excluding self) = at threshold, not over
        assert!(
            overloaded.is_empty(),
            "5 params (excluding self) should not exceed threshold"
        );
    }

    #[test]
    fn detects_low_cohesion() {
        let src = r#"
struct Foo;
impl Foo {
    fn not_using_self(&self) -> i32 {
        42
    }
}
"#;
        let findings = parse_and_check(src);
        let low_cohesion: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(
            !low_cohesion.is_empty(),
            "method that takes self but never uses it should be flagged"
        );
        assert!(low_cohesion[0].message.contains("not_using_self"));
    }

    #[test]
    fn no_low_cohesion_when_self_used() {
        let src = r#"
struct Foo { value: i32 }
impl Foo {
    fn get_value(&self) -> i32 {
        self.value
    }
}
"#;
        let findings = parse_and_check(src);
        let low_cohesion: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(
            low_cohesion.is_empty(),
            "method that uses self should not be flagged"
        );
    }

    #[test]
    fn skips_associated_functions() {
        let src = r#"
struct Foo;
impl Foo {
    fn new() -> Self {
        Foo
    }
}
"#;
        let findings = parse_and_check(src);
        let low_cohesion: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(
            low_cohesion.is_empty(),
            "associated function without self should not be flagged"
        );
    }

    #[test]
    fn correct_metadata() {
        let src = r#"
struct Foo;
impl Foo {
    fn orphan(&self) -> i32 {
        42
    }
}
"#;
        let findings = parse_and_check(src);
        let low_cohesion: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(!low_cohesion.is_empty());
        let f = &low_cohesion[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "coupling");
        assert_eq!(f.severity, "info");
    }
}
