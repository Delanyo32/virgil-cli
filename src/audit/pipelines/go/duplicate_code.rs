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
        "Detects duplicate function bodies and duplicate switch case arms"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── duplicate_function_body ──
        let groups = find_duplicate_bodies(
            root,
            source,
            &["function_declaration", "method_declaration"],
            "body",
            "name",
            5,
        );

        for group in &groups {
            if group.len() < 2 {
                continue;
            }
            let names: Vec<&str> = group.iter().map(|(name, _, _)| name.as_str()).collect();

            for (name, line, col) in group {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *line,
                    column: *col,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_function_body".to_string(),
                    message: format!(
                        "function `{name}` has a body identical to: {}",
                        names
                            .iter()
                            .filter(|n| **n != name.as_str())
                            .map(|n| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    snippet: String::new(),
                });
            }
        }

        // ── duplicate_switch_cases ──
        let switch_dups = find_duplicate_arms(
            root,
            source,
            "expression_switch_statement",
            "expression_case",
            None,
        );

        for (switch_line, dup_lines) in &switch_dups {
            for dup_line in dup_lines {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *dup_line,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_switch_case".to_string(),
                    message: format!(
                        "switch case at line {dup_line} has a duplicate body (switch at line {switch_line})"
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DuplicateCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    // ── duplicate_function_body ──

    #[test]
    fn detects_duplicate_function_bodies() {
        let src = r#"package main

func doA() int {
    x := 1
    y := 2
    z := x + y
    w := z * 2
    return w
}

func doB() int {
    x := 1
    y := 2
    z := x + y
    w := z * 2
    return w
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert_eq!(dups.len(), 2); // Both functions reported
    }

    #[test]
    fn clean_unique_function_bodies() {
        let src = r#"package main

func doA() int {
    x := 1
    y := 2
    z := x + y
    w := z * 2
    return w
}

func doB() string {
    a := "hello"
    b := " world"
    c := a + b
    d := c + "!"
    return d
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(dups.is_empty());
    }

    // ── duplicate_switch_case ──

    #[test]
    fn detects_duplicate_switch_cases() {
        let src = r#"package main

func classify(x int) string {
    switch x {
    case 1:
        result := "low"
        return result
    case 2:
        result := "medium"
        return result
    case 3:
        result := "low"
        return result
    }
    return "unknown"
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_case")
            .collect();
        assert!(!dups.is_empty());
    }

    #[test]
    fn clean_unique_switch_cases() {
        let src = r#"package main

func classify(x int) string {
    switch x {
    case 1:
        return "low"
    case 2:
        return "medium"
    case 3:
        return "high"
    }
    return "unknown"
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_case")
            .collect();
        assert!(dups.is_empty());
    }

    // ── metadata ──

    #[test]
    fn metadata_check() {
        let pipeline = DuplicateCodePipeline::new().unwrap();
        assert_eq!(pipeline.name(), "duplicate_code");
        assert!(!pipeline.description().is_empty());
    }
}
