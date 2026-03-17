use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    body_has_member_access, count_nodes_of_kind, count_parameters,
};

use super::primitives::extract_snippet;

const IMPORT_THRESHOLD: usize = 15;
const PARAMETER_THRESHOLD: usize = 5;

pub struct CouplingPipeline;

impl CouplingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Count `import_statement` nodes at the program root.
    /// If the count exceeds the threshold, flag the file.
    fn check_excessive_imports(
        root: tree_sitter::Node,
        _source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let count = count_nodes_of_kind(root, &["import_statement"]);
        if count > IMPORT_THRESHOLD {
            let start = root.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "info".to_string(),
                pipeline: "coupling".to_string(),
                pattern: "excessive_imports".to_string(),
                message: format!(
                    "file has {count} imports (threshold: {IMPORT_THRESHOLD}) — consider splitting into smaller modules"
                ),
                snippet: String::new(),
            });
        }
    }

    /// Find function/method nodes with too many parameters.
    fn check_parameter_overload(
        node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let kind = node.kind();
        let is_function = kind == "function_declaration"
            || kind == "arrow_function"
            || kind == "function_expression"
            || kind == "method_definition"
            || kind == "generator_function_declaration";

        if is_function {
            // Try both "parameters" and "formal_parameters" field names
            let params_node = node
                .child_by_field_name("parameters")
                .or_else(|| node.child_by_field_name("formal_parameters"));

            if let Some(params) = params_node {
                let count = count_parameters(params);
                if count > PARAMETER_THRESHOLD {
                    let start = node.start_position();
                    let name = node
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("<anonymous>");
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "coupling".to_string(),
                        pattern: "parameter_overload".to_string(),
                        message: format!(
                            "`{name}` has {count} parameters (threshold: {PARAMETER_THRESHOLD}) — consider using an options object"
                        ),
                        snippet: extract_snippet(source, node, 1),
                    });
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::check_parameter_overload(child, source, file_path, findings);
        }
    }

    /// Find `method_definition` nodes inside `class_body`.
    /// Check if the method body references "this" via member_expression.
    /// No "this" access = low cohesion flag.
    fn check_low_cohesion(
        node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        if node.kind() == "class_body" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "method_definition" {
                    // Skip constructor — it typically sets this.* fields
                    let method_name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("");
                    if method_name == "constructor" {
                        continue;
                    }

                    if let Some(body) = child.child_by_field_name("body") {
                        let uses_this =
                            body_has_member_access(body, source, "member_expression", "this");
                        if !uses_this {
                            let start = child.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "info".to_string(),
                                pipeline: "coupling".to_string(),
                                pattern: "low_cohesion".to_string(),
                                message: format!(
                                    "method `{method_name}` does not access `this` — consider making it a standalone function"
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
            Self::check_low_cohesion(child, source, file_path, findings);
        }
    }
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low class cohesion in JavaScript files"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        Self::check_excessive_imports(root, source, file_path, &mut findings);
        Self::check_parameter_overload(root, source, file_path, &mut findings);
        Self::check_low_cohesion(root, source, file_path, &mut findings);

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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    // ── excessive_imports ──────────────────────────────────────────

    #[test]
    fn detects_excessive_imports() {
        let mut src = String::new();
        for i in 0..16 {
            src.push_str(&format!("import {{ mod{i} }} from './mod{i}';\n"));
        }
        src.push_str("console.log('ok');\n");
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert_eq!(excessive.len(), 1);
        assert!(excessive[0].message.contains("16"));
    }

    #[test]
    fn no_excessive_imports_under_threshold() {
        let mut src = String::new();
        for i in 0..5 {
            src.push_str(&format!("import {{ mod{i} }} from './mod{i}';\n"));
        }
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert!(excessive.is_empty());
    }

    // ── parameter_overload ─────────────────────────────────────────

    #[test]
    fn detects_parameter_overload() {
        let src = "function foo(a, b, c, d, e, f) { return a; }";
        let findings = parse_and_check(src);
        let overload: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overload.len(), 1);
        assert!(overload[0].message.contains("6"));
    }

    #[test]
    fn no_parameter_overload_under_threshold() {
        let src = "function foo(a, b, c) { return a; }";
        let findings = parse_and_check(src);
        let overload: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overload.is_empty());
    }

    #[test]
    fn detects_arrow_function_parameter_overload() {
        let src = "const foo = (a, b, c, d, e, f, g) => a;";
        let findings = parse_and_check(src);
        let overload: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overload.len(), 1);
    }

    // ── low_cohesion ───────────────────────────────────────────────

    #[test]
    fn detects_low_cohesion_method() {
        let src = r#"class Foo {
    constructor() {
        this.x = 1;
    }
    bar() {
        console.log("no this");
    }
}"#;
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert_eq!(low.len(), 1);
        assert!(low[0].message.contains("bar"));
    }

    #[test]
    fn no_low_cohesion_when_using_this() {
        let src = r#"class Foo {
    bar() {
        return this.x + 1;
    }
}"#;
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_constructor_for_low_cohesion() {
        let src = r#"class Foo {
    constructor() {
        console.log("init");
    }
}"#;
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    // ── metadata ───────────────────────────────────────────────────

    #[test]
    fn pipeline_metadata() {
        let pipeline = CouplingPipeline::new().unwrap();
        assert_eq!(pipeline.name(), "coupling");
        assert!(!pipeline.description().is_empty());
    }
}
