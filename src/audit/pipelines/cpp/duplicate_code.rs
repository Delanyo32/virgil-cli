use std::collections::HashMap;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{find_duplicate_arms, hash_block_normalized};

use super::primitives::find_identifier_in_declarator;

const MIN_BODY_LINES: usize = 5;

pub struct DuplicateCodePipeline;

impl DuplicateCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_duplicate_function_bodies(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Manually walk for function_definition nodes and extract name from
        // the declarator chain (function_declarator -> declarator or qualified_identifier).
        let mut hash_map: HashMap<u64, Vec<(String, u32, u32)>> = HashMap::new();
        collect_function_hashes(root, source, MIN_BODY_LINES, &mut hash_map);

        for group in hash_map.values() {
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

        findings
    }

    fn check_duplicate_switch_cases(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let switch_dups = find_duplicate_arms(
            root,
            source,
            "switch_statement",
            "case_statement",
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

/// Recursively collect function_definition nodes, hash their bodies, and group by hash.
fn collect_function_hashes(
    node: tree_sitter::Node,
    source: &[u8],
    min_lines: usize,
    hash_map: &mut HashMap<u64, Vec<(String, u32, u32)>>,
) {
    if node.kind() == "function_definition" {
        if let Some(body) = node.child_by_field_name("body") {
            if body.kind() == "compound_statement" {
                let body_lines = body.end_position().row.saturating_sub(body.start_position().row) + 1;
                if body_lines >= min_lines {
                    let hash = hash_block_normalized(body, source);
                    let name = node
                        .child_by_field_name("declarator")
                        .and_then(|d| find_identifier_in_declarator(d, source))
                        .unwrap_or_else(|| "<anonymous>".to_string());
                    let pos = node.start_position();
                    hash_map
                        .entry(hash)
                        .or_default()
                        .push((name, pos.row as u32 + 1, pos.column as u32 + 1));
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_hashes(child, source, min_lines, hash_map);
    }
}

impl Pipeline for DuplicateCodePipeline {
    fn name(&self) -> &str {
        "duplicate_code"
    }

    fn description(&self) -> &str {
        "Detects duplicate function bodies and duplicate switch case arms in C++"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_duplicate_function_bodies(tree, source, file_path));
        findings.extend(self.check_duplicate_switch_cases(tree, source, file_path));
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DuplicateCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    // ── duplicate_function_body ──

    #[test]
    fn detects_duplicate_function_bodies() {
        let src = r#"
int doA() {
    int x = 1;
    int y = 2;
    int z = x + y;
    int w = z * 2;
    return w;
}

int doB() {
    int x = 1;
    int y = 2;
    int z = x + y;
    int w = z * 2;
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
int doA() {
    int x = 1;
    int y = 2;
    int z = x + y;
    int w = z * 2;
    return w;
}

int doB() {
    float a = 1.0;
    float b = 2.0;
    float c = a + b;
    float d = c * 2.0;
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
    fn skips_short_function_bodies() {
        let src = r#"
int doA() {
    return 1;
}

int doB() {
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

    // ── duplicate_switch_case ──

    #[test]
    fn detects_duplicate_switch_cases() {
        let src = r#"
int classify(int x) {
    switch (x) {
    case 1:
        int r = 10;
        return r;
    case 2:
        int s = 20;
        return s;
    case 3:
        int r = 10;
        return r;
    }
    return 0;
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
