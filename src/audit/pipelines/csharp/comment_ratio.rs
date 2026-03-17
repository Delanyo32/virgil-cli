use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{compute_comment_ratio, ControlFlowConfig};

fn csharp_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "for_each_statement",
            "while_statement",
            "do_statement",
            "switch_section",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "for_each_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause"],
        logical_operators: &["&&", "||"],
        binary_expression_kind: "binary_expression",
        ternary_kind: Some("conditional_expression"),
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
        "Detects files with too few or too many comments relative to code"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let config = csharp_config();
        let (comment_lines, code_lines) = compute_comment_ratio(tree.root_node(), source, &config);

        let total = comment_lines + code_lines;
        if total == 0 {
            return findings;
        }

        let ratio = comment_lines as f64 / total as f64;

        if ratio < 0.05 {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "comment_to_code_ratio".to_string(),
                pattern: "under_documented".to_string(),
                message: format!(
                    "File has a comment-to-code ratio of {:.2} ({} comment lines, {} code lines) — consider adding documentation",
                    ratio, comment_lines, code_lines
                ),
                snippet: String::new(),
            });
        } else if ratio > 0.60 {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "comment_to_code_ratio".to_string(),
                pattern: "over_documented".to_string(),
                message: format!(
                    "File has a comment-to-code ratio of {:.2} ({} comment lines, {} code lines) — excessive comments may indicate unclear code",
                    ratio, comment_lines, code_lines
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CommentToCodeRatioPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_under_documented() {
        let mut lines = Vec::new();
        for i in 0..30 {
            lines.push(format!("int x{} = {};", i, i));
        }
        let source = format!("class Foo {{\n{}\n}}", lines.join("\n"));
        let findings = parse_and_check(&source);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "under_documented");
        assert_eq!(findings[0].pipeline, "comment_to_code_ratio");
    }

    #[test]
    fn no_finding_for_well_documented() {
        let source = r#"
// This module handles user authentication
// It provides login and logout functionality
class Auth {
    int login = 1;
    int logout = 0;
    // Helper for session management
    int session = 0;
    // Validate token
    string token = "abc";
    int valid = 1;
    int expired = 0;
}
"#;
        let findings = parse_and_check(source);
        assert!(findings.is_empty());
    }
}
