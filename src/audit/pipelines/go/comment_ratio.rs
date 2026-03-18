use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{compute_comment_ratio, is_test_file, ControlFlowConfig};

const UNDER_DOCUMENTED_THRESHOLD: f64 = 0.05;
const OVER_DOCUMENTED_THRESHOLD: f64 = 0.60;

fn config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[],
        nesting_increments: &[],
        flat_increments: &[],
        logical_operators: &[],
        binary_expression_kind: "binary_expression",
        ternary_kind: None,
        comment_kinds: &["comment"],
    }
}

pub struct CommentToCodeRatioPipeline;

impl CommentToCodeRatioPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl Pipeline for CommentToCodeRatioPipeline {
    fn name(&self) -> &str {
        "comment_to_code_ratio"
    }

    fn description(&self) -> &str {
        "Detects files that are under-documented (<5% comments) or over-documented (>60% comments)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let cfg = config();
        let (comment_lines, code_lines) = compute_comment_ratio(tree.root_node(), source, &cfg);

        let total = comment_lines + code_lines;
        if total == 0 {
            return findings;
        }

        let ratio = comment_lines as f64 / total as f64;

        if ratio < UNDER_DOCUMENTED_THRESHOLD && code_lines > 0 {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "comment_to_code_ratio".to_string(),
                pattern: "under_documented".to_string(),
                message: format!(
                    "Comment-to-code ratio is {ratio:.2} ({comment_lines} comment lines, {code_lines} code lines) — consider adding documentation"
                ),
                snippet: String::new(),
            });
        }

        if ratio > OVER_DOCUMENTED_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "comment_to_code_ratio".to_string(),
                pattern: "over_documented".to_string(),
                message: format!(
                    "Comment-to-code ratio is {ratio:.2} ({comment_lines} comment lines, {code_lines} code lines) — comments may be excessive"
                ),
                snippet: String::new(),
            });
        }

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
        let pipeline = CommentToCodeRatioPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_under_documented() {
        // Many lines of code with no comments
        let mut lines = vec!["package main".to_string(), String::new(), "func main() {".to_string()];
        for i in 0..30 {
            lines.push(format!("    x{i} := {i}"));
        }
        lines.push("}".to_string());
        let source = lines.join("\n");
        let findings = parse_and_check(&source);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "under_documented");
        assert_eq!(findings[0].pipeline, "comment_to_code_ratio");
    }

    #[test]
    fn detects_over_documented() {
        let mut comments = String::new();
        for i in 0..20 {
            comments.push_str(&format!("// comment line {i}\n"));
        }
        let src = format!("package main\n{comments}func main() {{}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "over_documented"));
    }

    #[test]
    fn no_finding_for_well_documented() {
        let src = r#"package main

// This function handles user authentication.
// It provides login functionality.
func login() {
    x := 1
    y := 2
    z := 3
    w := 4
    v := 5
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
