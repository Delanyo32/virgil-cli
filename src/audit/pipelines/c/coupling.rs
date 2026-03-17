use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_nodes_of_kind, count_parameters};

use super::primitives::find_identifier_in_declarator;

const INCLUDE_THRESHOLD: usize = 15;
const PARAMETER_THRESHOLD: usize = 5;

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
        "Detects excessive includes and functions with too many parameters in C"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── excessive_includes ───────────────────────────────────────
        // Count top-level `preproc_include` nodes.
        let include_count = count_nodes_of_kind(root, &["preproc_include"]);

        if include_count > INCLUDE_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "excessive_includes".to_string(),
                message: format!(
                    "{include_count} #include directives (threshold: {INCLUDE_THRESHOLD}) \
                     -- consider forward declarations or splitting the file"
                ),
                snippet: format!("{include_count} includes"),
            });
        }

        // ── parameter_overload ───────────────────────────────────────
        // Find function_definition nodes, get the parameter_list from the
        // function_declarator, and count parameters.
        collect_parameter_overload_findings(
            root,
            source,
            file_path,
            self.name(),
            &mut findings,
        );

        findings
    }
}

/// Walk all `function_definition` nodes and flag those with too many parameters.
fn collect_parameter_overload_findings(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "function_definition" {
        // function_definition -> declarator (function_declarator) -> parameters (parameter_list)
        if let Some(declarator) = node.child_by_field_name("declarator") {
            let func_name = find_identifier_in_declarator(declarator, source)
                .unwrap_or_else(|| "<unknown>".to_string());

            if let Some(params) = declarator.child_by_field_name("parameters") {
                let param_count = count_parameters(params);
                if param_count > PARAMETER_THRESHOLD {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "parameter_overload".to_string(),
                        message: format!(
                            "function `{func_name}` has {param_count} parameters \
                             (threshold: {PARAMETER_THRESHOLD}) -- consider using a struct"
                        ),
                        snippet: String::new(),
                    });
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_parameter_overload_findings(child, source, file_path, pipeline_name, findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CouplingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    // ── excessive_includes ──

    #[test]
    fn detects_excessive_includes() {
        let mut src = String::new();
        for i in 0..16 {
            src.push_str(&format!("#include <header{i}.h>\n"));
        }
        src.push_str("int main(void) { return 0; }\n");
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_includes")
            .collect();
        assert_eq!(excessive.len(), 1);
        assert!(excessive[0].message.contains("16"));
    }

    #[test]
    fn no_finding_under_include_threshold() {
        let mut src = String::new();
        for i in 0..15 {
            src.push_str(&format!("#include <header{i}.h>\n"));
        }
        src.push_str("int main(void) { return 0; }\n");
        let findings = parse_and_check(&src);
        let excessive: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "excessive_includes")
            .collect();
        assert!(excessive.is_empty());
    }

    // ── parameter_overload ──

    #[test]
    fn detects_too_many_parameters() {
        let src = r#"
void many_params(int a, int b, int c, int d, int e, int f) {
    a = b + c + d + e + f;
}
"#;
        let findings = parse_and_check(src);
        let overload: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert_eq!(overload.len(), 1);
        assert!(overload[0].message.contains("many_params"));
        assert!(overload[0].message.contains("6"));
    }

    #[test]
    fn no_finding_under_parameter_threshold() {
        let src = r#"
int add(int a, int b, int c) {
    return a + b + c;
}
"#;
        let findings = parse_and_check(src);
        let overload: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overload.is_empty());
    }

    #[test]
    fn no_finding_for_five_parameters() {
        let src = r#"
int sum5(int a, int b, int c, int d, int e) {
    return a + b + c + d + e;
}
"#;
        let findings = parse_and_check(src);
        let overload: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "parameter_overload")
            .collect();
        assert!(overload.is_empty());
    }

    // ── metadata ──

    #[test]
    fn metadata_check() {
        let pipeline = CouplingPipeline::new().unwrap();
        assert_eq!(pipeline.name(), "coupling");
        assert!(!pipeline.description().is_empty());
    }
}
