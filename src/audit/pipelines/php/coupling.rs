use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_parameters;

use super::primitives::{extract_snippet, node_text};

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

        // Count use_declaration (namespace_use_declaration), include_expression,
        // include_once_expression, require_expression, require_once_expression
        // at the root level (program children)
        let mut count = 0;
        let mut first_import_node = None;
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            let kind = child.kind();
            if kind == "namespace_use_declaration" || kind == "expression_statement" {
                // For expression_statement, check if it wraps an include/require
                if kind == "expression_statement" {
                    let mut inner_cursor = child.walk();
                    let has_include = child.children(&mut inner_cursor).any(|c| {
                        matches!(
                            c.kind(),
                            "include_expression"
                                | "include_once_expression"
                                | "require_expression"
                                | "require_once_expression"
                        )
                    });
                    if has_include {
                        count += 1;
                        if first_import_node.is_none() {
                            first_import_node = Some(child);
                        }
                    }
                } else {
                    count += 1;
                    if first_import_node.is_none() {
                        first_import_node = Some(child);
                    }
                }
            }
        }

        if count > IMPORT_THRESHOLD
            && let Some(import_node) = first_import_node
        {
            let start = import_node.start_position();
            findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "excessive_imports".to_string(),
                    message: format!(
                        "file has {count} imports (threshold: {IMPORT_THRESHOLD}) — consider splitting into smaller modules"
                    ),
                    snippet: extract_snippet(source, import_node, 3),
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

        if (kind == "function_definition" || kind == "method_declaration")
            && let Some(params) = node.child_by_field_name("parameters")
        {
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
                            "function `{name}` has {param_count} parameters (threshold: {PARAM_THRESHOLD}) — consider using an options object or parameter object"
                        ),
                        snippet: extract_snippet(source, node, 1),
                    });
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
        if node.kind() == "class_declaration"
            && let Some(body) = node.child_by_field_name("body")
        {
            let mut body_cursor = body.walk();
            for child in body.named_children(&mut body_cursor) {
                if child.kind() != "method_declaration" {
                    continue;
                }

                // Skip static methods
                if has_static_modifier(child) {
                    continue;
                }

                // Skip constructors/destructors
                let method_name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source))
                    .unwrap_or("");
                if method_name.starts_with("__") {
                    continue;
                }

                if let Some(method_body) = child.child_by_field_name("body") {
                    // Check if body references $this
                    if !body_references_this(method_body, source) {
                        let start = child.start_position();
                        findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "info".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "low_cohesion".to_string(),
                                message: format!(
                                    "method `{method_name}` does not use `$this` — consider making it a static method or standalone function"
                                ),
                                snippet: extract_snippet(source, child, 1),
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

fn has_static_modifier(method: tree_sitter::Node) -> bool {
    let mut cursor = method.walk();
    for child in method.children(&mut cursor) {
        if child.kind() == "static_modifier" {
            return true;
        }
    }
    false
}

/// Check if a method body references `$this`.
/// In PHP tree-sitter, `$this` appears as a `variable_name` node with text "$this".
fn body_references_this(root: tree_sitter::Node, source: &[u8]) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "variable_name" && node.utf8_text(source).unwrap_or("") == "$this" {
            return true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low method cohesion in PHP"
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    // ── excessive_imports ──

    #[test]
    fn detects_excessive_imports() {
        let imports: Vec<String> = (0..16)
            .map(|i| format!("use App\\Models\\Model{i};"))
            .collect();
        let src = format!("<?php\n{}\n\nfunction main() {{}}\n", imports.join("\n"));
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
        let src = r#"<?php
use App\Models\User;
use App\Models\Post;

function main() {
    $u = new User();
    $p = new Post();
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
        let src = r#"<?php
function tooMany($a, $b, $c, $d, $e, $f) {
    return $a + $b + $c + $d + $e + $f;
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
        let src = r#"<?php
function ok($a, $b) {
    return $a + $b;
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overloaded.is_empty());
    }

    #[test]
    fn detects_method_parameter_overload() {
        let src = r#"<?php
class Svc {
    public function tooMany($a, $b, $c, $d, $e, $f) {
        return $a + $b + $c + $d + $e + $f;
    }
}
"#;
        let findings = parse_and_check(src);
        let overloaded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overloaded.len(), 1);
    }

    // ── low_cohesion ──

    #[test]
    fn detects_low_cohesion() {
        let src = r#"<?php
class Svc {
    private $name;

    public function doNothing() {
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
        let src = r#"<?php
class Svc {
    private $name;

    public function getName() {
        return $this->name;
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
        let src = r#"<?php
class Svc {
    public static function helper() {
        return 42;
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
    fn skips_constructors() {
        let src = r#"<?php
class Svc {
    public function __construct() {
        // constructor without $this is ok
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
