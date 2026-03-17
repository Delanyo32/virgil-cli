use std::sync::Arc;

use anyhow::Result;
use tree_sitter::{Query, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use super::primitives;

pub struct CloneDetectionPipeline {
    method_query: Arc<Query>,
}

impl CloneDetectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: primitives::compile_method_call_query()?,
        })
    }

    fn message_for_pattern(pattern: &str) -> &'static str {
        match pattern {
            "clone" => ".clone() call — consider borrowing or taking ownership instead",
            "to_owned" => ".to_owned() call — consider borrowing a &str instead",
            "to_string" => ".to_string() on a type that may already be a String — consider borrowing",
            _ => "potential unnecessary clone",
        }
    }
}

impl Pipeline for CloneDetectionPipeline {
    fn name(&self) -> &str {
        "clone_detection"
    }

    fn description(&self) -> &str {
        "Detects overuse of .clone(), .to_owned(), and .to_string() that may indicate unnecessary allocations"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        let method_matches = primitives::find_method_calls(
            tree,
            source,
            &self.method_query,
            &["clone", "to_owned", "to_string"],
        );

        for m in method_matches {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: m.line,
                column: m.column,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: m.name.clone(),
                message: Self::message_for_pattern(&m.name).to_string(),
                snippet: m.text,
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
        let pipeline = CloneDetectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_clone_calls() {
        let src = r#"
fn example() {
    let a = String::from("hello");
    let b = a.clone();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "clone");
    }

    #[test]
    fn detects_to_owned_and_to_string() {
        let src = r#"
fn example() {
    let a = "hello".to_owned();
    let b = "world".to_string();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);

        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"to_owned"));
        assert!(patterns.contains(&"to_string"));
    }

    #[test]
    fn clean_code_no_findings() {
        let src = r#"
fn example(s: &str) -> usize {
    let a = s.len();
    let b = s.is_empty();
    a + if b { 0 } else { 1 }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn findings_have_correct_metadata() {
        let src = r#"fn main() { let x = vec![1].clone(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "clone_detection");
        assert_eq!(f.pattern, "clone");
        assert_eq!(f.severity, "info");
        assert_eq!(
            f.message,
            ".clone() call — consider borrowing or taking ownership instead"
        );
    }

    #[test]
    fn snippet_captures_full_expression() {
        let src = r#"fn main() { let x = vec![1].clone(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].snippet.contains("vec![1].clone()"));
    }
}
