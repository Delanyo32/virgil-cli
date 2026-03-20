use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{find_duplicate_arms, find_duplicate_bodies};

pub struct DuplicateCodePipeline;

impl DuplicateCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl Pipeline for DuplicateCodePipeline {
    fn name(&self) -> &str {
        "duplicate_code"
    }

    fn description(&self) -> &str {
        "Detects duplicate function bodies and duplicate match arms in Rust"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── duplicate_function_body ────────────────────────────────────
        let groups = find_duplicate_bodies(root, source, &["function_item"], "body", "name", 5);
        for group in &groups {
            let names: Vec<&str> = group.iter().map(|(n, _, _)| n.as_str()).collect();
            let label = names.join(", ");
            for (name, line, col) in group {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *line,
                    column: *col,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_function_body".to_string(),
                    message: format!(
                        "function `{name}` has a body identical to other function(s): {label} — consider extracting shared logic"
                    ),
                    snippet: String::new(),
                });
            }
        }

        // ── duplicate_match_arms ───────────────────────────────────────
        // In tree-sitter-rust, match_arm has a `value` field for the body expression.
        // We hash only the body so arms with different patterns but identical bodies
        // are flagged as duplicates.
        let dup_arms =
            find_duplicate_arms(root, source, "match_expression", "match_arm", Some("value"));
        for (match_line, dup_lines) in &dup_arms {
            for dup_line in dup_lines {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *dup_line,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_match_arm".to_string(),
                    message: format!(
                        "match arm at line {dup_line} is a duplicate of another arm in the match at line {match_line} — consider merging patterns"
                    ),
                    snippet: String::new(),
                });
            }
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
        let pipeline = DuplicateCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_duplicate_function_bodies() {
        let src = r#"
fn process_a(x: i32) -> i32 {
    let result = x * 2;
    let adjusted = result + 10;
    let normalized = adjusted / 3;
    let clamped = if normalized > 100 { 100 } else { normalized };
    clamped
}

fn process_b(x: i32) -> i32 {
    let result = x * 2;
    let adjusted = result + 10;
    let normalized = adjusted / 3;
    let clamped = if normalized > 100 { 100 } else { normalized };
    clamped
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(
            dups.len() >= 2,
            "should find at least 2 entries for the duplicate pair, found {}",
            dups.len()
        );
    }

    #[test]
    fn no_duplicates_in_different_bodies() {
        let src = r#"
fn add(a: i32, b: i32) -> i32 {
    let sum = a + b;
    let result = sum * 2;
    let adjusted = result + 1;
    let final_val = adjusted - 3;
    final_val
}

fn multiply(a: i32, b: i32) -> i32 {
    let product = a * b;
    let result = product / 2;
    let adjusted = result - 1;
    let final_val = adjusted + 3;
    final_val
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(
            dups.is_empty(),
            "different function bodies should not be flagged"
        );
    }

    #[test]
    fn detects_duplicate_match_arms() {
        let src = r#"
fn example(x: i32) {
    match x {
        1 => println!("hello"),
        2 => println!("hello"),
        3 => println!("world"),
    }
}
"#;
        let findings = parse_and_check(src);
        let dup_arms: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_match_arm")
            .collect();
        assert!(!dup_arms.is_empty(), "should detect duplicate match arms");
    }

    #[test]
    fn no_duplicate_match_arms_when_unique() {
        let src = r#"
fn example(x: i32) {
    match x {
        1 => println!("one"),
        2 => println!("two"),
        3 => println!("three"),
    }
}
"#;
        let findings = parse_and_check(src);
        let dup_arms: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_match_arm")
            .collect();
        assert!(
            dup_arms.is_empty(),
            "unique match arms should not be flagged"
        );
    }

    #[test]
    fn correct_metadata() {
        let src = r#"
fn dup_a(x: i32) -> i32 {
    let result = x * 2;
    let adjusted = result + 10;
    let normalized = adjusted / 3;
    let clamped = if normalized > 100 { 100 } else { normalized };
    clamped
}

fn dup_b(x: i32) -> i32 {
    let result = x * 2;
    let adjusted = result + 10;
    let normalized = adjusted / 3;
    let clamped = if normalized > 100 { 100 } else { normalized };
    clamped
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(!dups.is_empty());
        let f = &dups[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "duplicate_code");
        assert_eq!(f.severity, "warning");
    }
}
