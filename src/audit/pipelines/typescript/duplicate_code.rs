use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{find_duplicate_arms, find_duplicate_bodies};
use crate::audit::primitives::extract_snippet;
use crate::language::Language;

pub struct DuplicateCodePipeline {
    _language: Language,
}

impl DuplicateCodePipeline {
    pub fn new(language: Language) -> Result<Self> {
        // Validate the language can produce a tree-sitter language
        let _ts_lang = language.tree_sitter_language();
        Ok(Self {
            _language: language,
        })
    }
}

impl Pipeline for DuplicateCodePipeline {
    fn name(&self) -> &str {
        "duplicate_code"
    }

    fn description(&self) -> &str {
        "Detects duplicate function bodies and duplicate switch cases"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── duplicate_function_body ─────────────────────────────────
        let func_kinds = [
            "function_declaration",
            "arrow_function",
            "method_definition",
        ];
        let groups = find_duplicate_bodies(root, source, &func_kinds, "body", "name", 5);

        for group in groups {
            let names: Vec<&str> = group.iter().map(|(n, _, _)| n.as_str()).collect();
            let summary = names.join(", ");
            for (name, line, col) in &group {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *line,
                    column: *col,
                    severity: "warning".to_string(),
                    pipeline: "duplicate_code".to_string(),
                    pattern: "duplicate_function_body".to_string(),
                    message: format!(
                        "Function `{}` has a duplicate body shared with: [{}]",
                        name, summary
                    ),
                    snippet: find_snippet_at_line(root, source, *line),
                });
            }
        }

        // ── duplicate_switch_cases ──────────────────────────────────
        let dup_arms = find_duplicate_arms(root, source, "switch_statement", "switch_case", None);

        for (switch_line, dup_lines) in dup_arms {
            for dup_line in dup_lines {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: dup_line,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: "duplicate_code".to_string(),
                    pattern: "duplicate_switch_cases".to_string(),
                    message: format!(
                        "Switch case at line {} has a duplicate body (switch at line {})",
                        dup_line, switch_line
                    ),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

/// Find a node that starts near the given line and extract a snippet from it.
fn find_snippet_at_line(
    root: tree_sitter::Node,
    source: &[u8],
    target_line: u32,
) -> String {
    let target_row = target_line.saturating_sub(1) as usize;
    let mut best: Option<tree_sitter::Node> = None;

    let mut cursor = root.walk();
    find_node_at_line(root, &mut cursor, target_row, &mut best);

    best.map(|n| extract_snippet(source, n, 3))
        .unwrap_or_default()
}

fn find_node_at_line<'a>(
    node: tree_sitter::Node<'a>,
    _cursor: &mut tree_sitter::TreeCursor<'a>,
    target_row: usize,
    best: &mut Option<tree_sitter::Node<'a>>,
) {
    if node.start_position().row == target_row {
        *best = Some(node);
        return;
    }
    let mut child_cursor = node.walk();
    let children: Vec<_> = node.children(&mut child_cursor).collect();
    for child in children {
        if child.start_position().row <= target_row && child.end_position().row >= target_row {
            let mut inner = child.walk();
            find_node_at_line(child, &mut inner, target_row, best);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DuplicateCodePipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_duplicate_function_bodies() {
        // Two functions with identical bodies (>= 5 lines)
        let source = r#"
function foo(a: number): number {
    const x = a + 1;
    const y = x * 2;
    const z = y - 3;
    console.log(z);
    return z;
}

function bar(b: number): number {
    const x = b + 1;
    const y = x * 2;
    const z = y - 3;
    console.log(z);
    return z;
}
"#;
        let findings = parse_and_check(source);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert_eq!(dups.len(), 2);
        assert_eq!(dups[0].severity, "warning");
    }

    #[test]
    fn no_finding_for_different_function_bodies() {
        let source = r#"
function foo(): number {
    const x = 1;
    const y = 2;
    const z = 3;
    const w = 4;
    return x + y + z + w;
}

function bar(): string {
    const a = "hello";
    const b = "world";
    const c = a + b;
    const d = c.toUpperCase();
    return d;
}
"#;
        let findings = parse_and_check(source);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(dups.is_empty());
    }

    #[test]
    fn detects_duplicate_switch_cases() {
        let source = r#"
function test(x: number): string {
    switch (x) {
        case 1:
            console.log("hello");
            return "a";
        case 2:
            console.log("hello");
            return "a";
        case 3:
            return "c";
    }
}
"#;
        let findings = parse_and_check(source);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_cases")
            .collect();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].severity, "warning");
    }

    #[test]
    fn no_finding_for_unique_switch_cases() {
        let source = r#"
function test(x: number): string {
    switch (x) {
        case 1:
            return "a";
        case 2:
            return "b";
        case 3:
            return "c";
    }
}
"#;
        let findings = parse_and_check(source);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_cases")
            .collect();
        assert!(dups.is_empty());
    }

    #[test]
    fn no_finding_for_short_duplicate_functions() {
        // Bodies are < 5 lines, so should not be flagged
        let source = r#"
function foo(): number {
    return 1;
}

function bar(): number {
    return 1;
}
"#;
        let findings = parse_and_check(source);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(dups.is_empty());
    }

    #[test]
    fn metadata_is_correct() {
        let pipeline = DuplicateCodePipeline::new(Language::TypeScript).unwrap();
        assert_eq!(pipeline.name(), "duplicate_code");
        assert!(!pipeline.description().is_empty());
    }
}
