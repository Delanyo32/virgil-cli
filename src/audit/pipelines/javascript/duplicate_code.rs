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
        "Detects duplicate function bodies and duplicate switch cases in JavaScript files"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── duplicate_function_body ────────────────────────────────
        let func_kinds = [
            "function_declaration",
            "arrow_function",
            "method_definition",
        ];
        let groups = find_duplicate_bodies(root, source, &func_kinds, "body", "name", 5);

        for group in &groups {
            let names: Vec<&str> = group.iter().map(|(n, _, _)| n.as_str()).collect();
            let label = names.join(", ");
            for (name, line, column) in group {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *line,
                    column: *column,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_function_body".to_string(),
                    message: format!(
                        "`{name}` has a body identical to other function(s): [{label}] — consider extracting shared logic"
                    ),
                    snippet: String::new(),
                });
            }
        }

        // ── duplicate_switch_cases ─────────────────────────────────
        let switch_dups = find_duplicate_arms(root, source, "switch_statement", "switch_case", None);

        for (switch_line, dup_lines) in &switch_dups {
            for dup_line in dup_lines {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *dup_line,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_switch_cases".to_string(),
                    message: format!(
                        "switch case at line {dup_line} has a body identical to another case in switch at line {switch_line} — consider merging"
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DuplicateCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    // ── duplicate_function_body ────────────────────────────────────

    #[test]
    fn detects_duplicate_function_bodies() {
        let src = r#"function foo() {
    const a = 1;
    const b = 2;
    const c = 3;
    const d = 4;
    return a + b + c + d;
}

function bar() {
    const a = 1;
    const b = 2;
    const c = 3;
    const d = 4;
    return a + b + c + d;
}"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert_eq!(dups.len(), 2);
    }

    #[test]
    fn no_duplicate_for_different_bodies() {
        let src = r#"function foo() {
    const a = 1;
    const b = 2;
    const c = 3;
    const d = 4;
    return a + b + c + d;
}

function bar() {
    const x = 10;
    const y = 20;
    console.log(x);
    console.log(y);
    console.log(x + y);
    return x * y;
}"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(dups.is_empty());
    }

    // ── duplicate_switch_cases ─────────────────────────────────────

    #[test]
    fn detects_duplicate_switch_cases() {
        let src = r#"switch (x) {
    case 1:
        console.log("hello");
        break;
    case 2:
        console.log("hello");
        break;
    case 3:
        console.log("world");
        break;
}"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_cases")
            .collect();
        assert_eq!(dups.len(), 1);
    }

    #[test]
    fn no_duplicate_for_unique_cases() {
        let src = r#"switch (x) {
    case 1:
        console.log("a");
        break;
    case 2:
        console.log("b");
        break;
    case 3:
        console.log("c");
        break;
}"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_cases")
            .collect();
        assert!(dups.is_empty());
    }

    // ── metadata ───────────────────────────────────────────────────

    #[test]
    fn pipeline_metadata() {
        let pipeline = DuplicateCodePipeline::new().unwrap();
        assert_eq!(pipeline.name(), "duplicate_code");
        assert!(!pipeline.description().is_empty());
    }
}
