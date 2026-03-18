use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{compute_comment_ratio, is_test_file, ControlFlowConfig};

fn php_config() -> ControlFlowConfig {
    ControlFlowConfig {
        decision_point_kinds: &[
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "do_statement",
            "case_statement",
            "catch_clause",
        ],
        nesting_increments: &[
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "do_statement",
            "switch_statement",
            "catch_clause",
        ],
        flat_increments: &["else_clause", "else_if_clause"],
        logical_operators: &["&&", "||", "and", "or"],
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
        "Checks comment-to-code ratio in PHP files"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let config = php_config();
        let (comment_lines, code_lines) = compute_comment_ratio(tree.root_node(), source, &config);

        if code_lines == 0 {
            return findings;
        }

        let ratio = comment_lines as f64 / (comment_lines + code_lines) as f64;

        if ratio < 0.05 {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: self.name().to_string(),
                pattern: "under_documented".to_string(),
                message: format!(
                    "comment-to-code ratio is {:.2} ({comment_lines} comment lines / {} total non-blank lines) — consider adding documentation",
                    ratio,
                    comment_lines + code_lines
                ),
                snippet: String::new(),
            });
        } else if ratio > 0.60 {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: self.name().to_string(),
                pattern: "over_documented".to_string(),
                message: format!(
                    "comment-to-code ratio is {:.2} ({comment_lines} comment lines / {} total non-blank lines) — excessive comments may indicate dead code or noise",
                    ratio,
                    comment_lines + code_lines
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CommentToCodeRatioPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_under_documented() {
        let mut lines = vec!["<?php".to_string()];
        for i in 0..30 {
            lines.push(format!("$x{i} = {i};"));
        }
        lines.push("?>".to_string());
        let src = lines.join("\n");
        let findings = parse_and_check(&src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].pattern, "under_documented");
    }

    #[test]
    fn clean_well_documented() {
        let src = r#"<?php
// This is a helper module
// It provides utility functions
$x = 1;
$y = 2;
// Calculate result
$z = $x + $y;
// Return
echo $z;
?>"#;
        let findings = parse_and_check(src);
        // Ratio should be moderate, not triggering either threshold
        let under = findings.iter().any(|f| f.pattern == "under_documented");
        let over = findings.iter().any(|f| f.pattern == "over_documented");
        assert!(!under);
        assert!(!over);
    }
}
