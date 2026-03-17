use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{
    body_has_member_access, count_nodes_of_kind, count_parameters,
};
use crate::audit::primitives::{extract_snippet, node_text};
use crate::language::Language;

pub struct CouplingPipeline {
    _language: Language,
}

impl CouplingPipeline {
    pub fn new(language: Language) -> Result<Self> {
        // Validate the language can produce a tree-sitter language
        let _ts_lang = language.tree_sitter_language();
        Ok(Self {
            _language: language,
        })
    }
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low class cohesion"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── excessive_imports ───────────────────────────────────────
        let import_count = count_nodes_of_kind(root, &["import_statement"]);
        if import_count > 15 {
            let start = root.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "info".to_string(),
                pipeline: "coupling".to_string(),
                pattern: "excessive_imports".to_string(),
                message: format!(
                    "File has {} import statements (threshold: 15) — consider splitting this module",
                    import_count
                ),
                snippet: String::new(),
            });
        }

        // ── parameter_overload ──────────────────────────────────────
        check_parameter_overload(root, source, file_path, &mut findings);

        // ── low_cohesion ────────────────────────────────────────────
        check_low_cohesion(root, source, file_path, &mut findings);

        findings
    }
}

/// Walk the tree to find functions/methods and check parameter count.
fn check_parameter_overload(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let kind = node.kind();
    let is_func = kind == "function_declaration"
        || kind == "arrow_function"
        || kind == "method_definition";

    if is_func {
        // Try both "parameters" and "formal_parameters" field names
        let params_node = node
            .child_by_field_name("parameters")
            .or_else(|| node.child_by_field_name("formal_parameters"));

        if let Some(params) = params_node {
            let param_count = count_parameters(params);
            if param_count > 5 {
                let fn_name = node
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source))
                    .unwrap_or("<anonymous>");
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: "coupling".to_string(),
                    pattern: "parameter_overload".to_string(),
                    message: format!(
                        "Function `{}` has {} parameters (threshold: 5) — consider using an options object",
                        fn_name, param_count
                    ),
                    snippet: extract_snippet(source, node, 3),
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        check_parameter_overload(child, source, file_path, findings);
    }
}

/// Find method_definition nodes inside class_body inside class_declaration.
/// Check if the method body references `this` via member_expression.
fn check_low_cohesion(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "class_declaration" {
        let class_name = node
            .child_by_field_name("name")
            .map(|n| node_text(n, source))
            .unwrap_or("<anonymous>");

        // Find class_body child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_body" {
                let mut body_cursor = child.walk();
                for member in child.children(&mut body_cursor) {
                    if member.kind() == "method_definition" {
                        let method_name = member
                            .child_by_field_name("name")
                            .map(|n| node_text(n, source))
                            .unwrap_or("<anonymous>");

                        // Skip constructor — it typically assigns to this
                        if method_name == "constructor" {
                            continue;
                        }

                        if let Some(body) = member.child_by_field_name("body") {
                            let uses_this = body_has_member_access(
                                body,
                                source,
                                "member_expression",
                                "this",
                            );
                            if !uses_this {
                                let start = member.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "info".to_string(),
                                    pipeline: "coupling".to_string(),
                                    pattern: "low_cohesion".to_string(),
                                    message: format!(
                                        "Method `{}` in class `{}` does not access `this` — consider making it a standalone function",
                                        method_name, class_name
                                    ),
                                    snippet: extract_snippet(source, member, 3),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Recurse into children (to find nested classes etc.)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        check_low_cohesion(child, source, file_path, findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_excessive_imports() {
        let mut lines: Vec<String> = Vec::new();
        for i in 0..16 {
            lines.push(format!("import {{ mod{} }} from './mod{}';", i, i));
        }
        lines.push("console.log('done');".to_string());
        let source = lines.join("\n");
        let findings = parse_and_check(&source);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert_eq!(excessive.len(), 1);
        assert_eq!(excessive[0].severity, "info");
        assert!(excessive[0].message.contains("16"));
    }

    #[test]
    fn no_finding_for_few_imports() {
        let source = r#"
import { a } from './a';
import { b } from './b';
import { c } from './c';
console.log(a, b, c);
"#;
        let findings = parse_and_check(source);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_imports")
            .collect();
        assert!(excessive.is_empty());
    }

    #[test]
    fn detects_parameter_overload() {
        let source = r#"
function createUser(name: string, age: number, email: string, phone: string, address: string, role: string): void {
    console.log(name, age, email, phone, address, role);
}
"#;
        let findings = parse_and_check(source);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloads.len(), 1);
        assert_eq!(overloads[0].severity, "warning");
        assert!(overloads[0].message.contains("createUser"));
        assert!(overloads[0].message.contains("6"));
    }

    #[test]
    fn no_finding_for_few_parameters() {
        let source = r#"
function add(a: number, b: number): number {
    return a + b;
}
"#;
        let findings = parse_and_check(source);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloads.is_empty());
    }

    #[test]
    fn detects_low_cohesion() {
        let source = r#"
class Utils {
    static helper(x: number): number {
        return x + 1;
    }

    format(value: string): string {
        return value.trim();
    }
}
"#;
        let findings = parse_and_check(source);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        // Both methods don't use `this`
        assert!(low.len() >= 1);
        assert_eq!(low[0].severity, "info");
    }

    #[test]
    fn no_finding_when_method_uses_this() {
        let source = r#"
class Counter {
    count: number = 0;

    increment(): void {
        this.count += 1;
    }

    getCount(): number {
        return this.count;
    }
}
"#;
        let findings = parse_and_check(source);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert!(low.is_empty());
    }

    #[test]
    fn skips_constructor_for_low_cohesion() {
        let source = r#"
class Greeter {
    name: string;

    constructor(name: string) {
        // constructors often just assign, not accessing this.x via member_expression
    }

    greet(): string {
        return this.name;
    }
}
"#;
        let findings = parse_and_check(source);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        // constructor is skipped, greet uses this — no findings
        assert!(low.is_empty());
    }

    #[test]
    fn detects_method_parameter_overload() {
        let source = r#"
class Service {
    process(a: number, b: string, c: boolean, d: number, e: string, f: boolean): void {
        this.log(a, b, c, d, e, f);
    }

    log(...args: any[]): void {
        console.log(args);
    }
}
"#;
        let findings = parse_and_check(source);
        let overloads: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloads.len(), 1);
        assert!(overloads[0].message.contains("process"));
    }

    #[test]
    fn metadata_is_correct() {
        let pipeline = CouplingPipeline::new(Language::TypeScript).unwrap();
        assert_eq!(pipeline.name(), "coupling");
        assert!(!pipeline.description().is_empty());
    }
}
