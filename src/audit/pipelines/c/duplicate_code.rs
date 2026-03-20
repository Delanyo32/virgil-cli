use std::collections::HashMap;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{find_duplicate_arms, hash_block_normalized};

use super::primitives::find_identifier_in_declarator;

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
        "Detects duplicate function bodies and duplicate switch case arms in C"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── duplicate_function_body ─────────────────────────────────
        // C function_definition has a `declarator` field (function_declarator)
        // and a `body` field (compound_statement). The name is buried inside
        // the declarator chain, so we walk manually rather than using
        // find_duplicate_bodies which expects a flat `name` field.
        let mut hash_map: HashMap<u64, Vec<(String, u32, u32)>> = HashMap::new();
        collect_function_hashes(root, source, 5, &mut hash_map);

        let groups: Vec<_> = hash_map
            .into_values()
            .filter(|group| group.len() >= 2)
            .collect();

        for group in &groups {
            let names: Vec<&str> = group.iter().map(|(n, _, _)| n.as_str()).collect();
            for (name, line, col) in group {
                let others: Vec<String> = names
                    .iter()
                    .filter(|n| **n != name.as_str())
                    .map(|n| format!("`{n}`"))
                    .collect();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: *line,
                    column: *col,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_function_body".to_string(),
                    message: format!(
                        "function `{name}` has a body identical to: {}",
                        others.join(", ")
                    ),
                    snippet: String::new(),
                });
            }
        }

        // ── duplicate_switch_cases ──────────────────────────────────
        let switch_dups =
            find_duplicate_arms(root, source, "switch_statement", "case_statement", None);

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
                        "switch case at line {} has a duplicate body (switch at line {})",
                        dup_line, switch_line
                    ),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

/// Recursively find all `function_definition` nodes, extract the name from the
/// declarator chain, hash the body, and group by hash.
fn collect_function_hashes(
    node: tree_sitter::Node,
    source: &[u8],
    min_lines: usize,
    hash_map: &mut HashMap<u64, Vec<(String, u32, u32)>>,
) {
    if node.kind() == "function_definition"
        && let Some(body) = node.child_by_field_name("body") {
            let body_lines = body
                .end_position()
                .row
                .saturating_sub(body.start_position().row)
                + 1;
            if body_lines >= min_lines {
                let name = node
                    .child_by_field_name("declarator")
                    .and_then(|d| find_identifier_in_declarator(d, source))
                    .unwrap_or_else(|| "<anonymous>".to_string());
                let hash = hash_block_normalized(body, source);
                let pos = node.start_position();
                hash_map.entry(hash).or_default().push((
                    name,
                    pos.row as u32 + 1,
                    pos.column as u32 + 1,
                ));
            }
        }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_hashes(child, source, min_lines, hash_map);
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
        let pipeline = DuplicateCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    // ── duplicate_function_body ──

    #[test]
    fn detects_duplicate_function_bodies() {
        let src = r#"
int do_a(int n) {
    int x = n + 1;
    int y = x * 2;
    int z = y - 3;
    int w = z + 4;
    return w;
}

int do_b(int m) {
    int x = m + 1;
    int y = x * 2;
    int z = y - 3;
    int w = z + 4;
    return w;
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
        let src = r#"
int do_a(int n) {
    int x = n + 1;
    int y = x * 2;
    int z = y - 3;
    int w = z + 4;
    return w;
}

int do_b(int m) {
    int a = m * 10;
    int b = a / 3;
    int c = b + 7;
    int d = c - 1;
    return d;
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(dups.is_empty());
    }

    #[test]
    fn no_finding_for_short_duplicate_functions() {
        // Bodies are < 5 lines, should not be flagged
        let src = r#"
int do_a(void) {
    return 1;
}

int do_b(void) {
    return 1;
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(dups.is_empty());
    }

    // ── duplicate_switch_cases ──

    #[test]
    fn detects_duplicate_switch_cases() {
        let src = r#"
int classify(int x) {
    switch (x) {
        case 1:
            x = x + 1;
            return x;
        case 2:
            x = x * 2;
            return x;
        case 3:
            x = x + 1;
            return x;
    }
    return 0;
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_cases")
            .collect();
        assert!(!dups.is_empty());
    }

    #[test]
    fn clean_unique_switch_cases() {
        let src = r#"
int classify(int x) {
    switch (x) {
        case 1:
            return 10;
        case 2:
            return 20;
        case 3:
            return 30;
    }
    return 0;
}
"#;
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_switch_cases")
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
