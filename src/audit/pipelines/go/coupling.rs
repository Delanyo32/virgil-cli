use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{body_references_identifier, count_parameters};

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

        // Count all import_spec nodes in the file
        let count = count_import_specs(root);

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
                            "file has {count} imports (threshold: {IMPORT_THRESHOLD}) — consider splitting into smaller packages"
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

fn count_import_specs(root: tree_sitter::Node) -> usize {
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "import_spec" {
            count += 1;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    count
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

        if (kind == "function_declaration" || kind == "method_declaration")
            && let Some(params) = node.child_by_field_name("parameters")
        {
            let param_count = count_parameters(params);
            if param_count > PARAM_THRESHOLD {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source))
                    .unwrap_or("<anonymous>");

                // Skip constructor functions (Go convention: New*)
                if !name.starts_with("New") {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "parameter_overload".to_string(),
                            message: format!(
                                "function `{name}` has {param_count} parameters (threshold: {PARAM_THRESHOLD}) — consider using an options struct"
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
        if node.kind() == "method_declaration" {
            // Go method: func (r ReceiverType) Name(params) { body }
            // The receiver is in the "receiver" field which is a parameter_list
            if let Some(receiver_list) = node.child_by_field_name("receiver") {
                // The receiver parameter_list contains parameter_declaration(s)
                // Typically just one: (r *ReceiverType)
                let mut recv_cursor = receiver_list.walk();
                let receiver_param = receiver_list.named_children(&mut recv_cursor).next();

                if let Some(param) = receiver_param {
                    // The parameter_declaration has a name field (the receiver variable name)
                    if let Some(name_node) = param.child_by_field_name("name") {
                        let receiver_name = node_text(name_node, source);

                        if !receiver_name.is_empty()
                            && let Some(body) = node.child_by_field_name("body")
                            && !body_references_identifier(body, source, receiver_name)
                        {
                            let method_name = node
                                .child_by_field_name("name")
                                .map(|n| node_text(n, source))
                                .unwrap_or("<anonymous>");

                            let start = node.start_position();
                            findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "info".to_string(),
                                        pipeline: pipeline_name.to_string(),
                                        pattern: "low_cohesion".to_string(),
                                        message: format!(
                                            "method `{method_name}` does not use its receiver `{receiver_name}` — consider making it a function"
                                        ),
                                        snippet: extract_snippet(source, node, 1),
                                    });
                        }
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

impl Pipeline for CouplingPipeline {
    fn name(&self) -> &str {
        "coupling"
    }

    fn description(&self) -> &str {
        "Detects excessive imports, parameter overload, and low method cohesion"
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    // ── excessive_imports ──

    #[test]
    fn detects_excessive_imports() {
        let imports: Vec<String> = (0..16).map(|i| format!("\"pkg{i}\"")).collect();
        let src = format!(
            "package main\n\nimport (\n{}\n)\n\nfunc main() {{}}\n",
            imports.join("\n")
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
        let src = r#"package main

import (
    "fmt"
    "os"
)

func main() {
    fmt.Println(os.Args)
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
        let src = r#"package main

func tooMany(a int, b int, c int, d int, e int, f int) int {
    return a + b + c + d + e + f
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
        let src = r#"package main

func ok(a int, b int) int {
    return a + b
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
        let src = r#"package main

type Svc struct {
    Name string
}

func (s *Svc) DoNothing() string {
    return "hello"
}
"#;
        let findings = parse_and_check(src);
        let low: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "low_cohesion")
            .collect();
        assert_eq!(low.len(), 1);
        assert!(low[0].message.contains("DoNothing"));
        assert!(low[0].message.contains("s"));
    }

    #[test]
    fn clean_cohesive_method() {
        let src = r#"package main

type Svc struct {
    Name string
}

func (s *Svc) GetName() string {
    return s.Name
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
