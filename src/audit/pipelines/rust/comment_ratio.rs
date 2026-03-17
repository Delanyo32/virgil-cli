use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::complexity_helpers::{compute_comment_ratio, ControlFlowConfig};

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
        comment_kinds: &["line_comment", "block_comment"],
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CommentToCodeRatioPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_under_documented() {
        // Many lines of code with no comments
        let mut lines = Vec::new();
        for i in 0..30 {
            lines.push(format!("let x{i} = {i};"));
        }
        let source = format!("fn main() {{\n{}\n}}", lines.join("\n"));
        let findings = parse_and_check(&source);
        assert!(findings.iter().any(|f| f.pattern == "under_documented"));
        assert_eq!(findings[0].pipeline, "comment_to_code_ratio");
    }

    #[test]
    fn detects_over_documented() {
        let mut comments = String::new();
        for i in 0..20 {
            comments.push_str(&format!("// comment line {i}\n"));
        }
        let source = format!("{comments}fn main() {{}}\n");
        let findings = parse_and_check(&source);
        assert!(findings.iter().any(|f| f.pattern == "over_documented"));
    }

    #[test]
    fn clean_well_documented() {
        let src = r#"
// This module handles user data.
// It provides methods for CRUD operations.
fn create() {}
fn read() {}
fn update() {}
fn delete() {}
fn validate() {}
fn serialize() {}
fn deserialize() {}
fn transform() {}
fn filter() {}
fn sort() {}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
