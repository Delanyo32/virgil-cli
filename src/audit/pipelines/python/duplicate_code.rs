use std::collections::HashMap;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{find_duplicate_bodies, hash_block_normalized};

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

        let groups =
            find_duplicate_bodies(root, source, &["function_definition"], "body", "name", 5);

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

        findings
    }

    fn check_duplicate_elif_branches(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        collect_duplicate_branches(root, source, file_path, self.name(), &mut findings);

        findings
    }
}

/// Walk the tree looking for if_statement nodes, then collect all branch bodies
/// (consequence, elif_clause bodies, else_clause body), hash them, and flag duplicates.
fn collect_duplicate_branches(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "if_statement" {
        let mut branch_hashes: Vec<(u64, u32)> = Vec::new();

        // Collect the consequence block (the if-body)
        if let Some(consequence) = node.child_by_field_name("consequence") {
            let hash = hash_block_normalized(consequence, source);
            branch_hashes.push((hash, consequence.start_position().row as u32 + 1));
        }

        // Walk children looking for elif_clause and else_clause
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "elif_clause" => {
                    if let Some(body) = child.child_by_field_name("consequence") {
                        let hash = hash_block_normalized(body, source);
                        branch_hashes.push((hash, child.start_position().row as u32 + 1));
                    }
                }
                "else_clause" => {
                    if let Some(body) = child.child_by_field_name("body") {
                        let hash = hash_block_normalized(body, source);
                        branch_hashes.push((hash, child.start_position().row as u32 + 1));
                    }
                }
                _ => {}
            }
        }

        // Detect duplicates among branches
        if branch_hashes.len() >= 2 {
            let mut seen: HashMap<u64, u32> = HashMap::new();
            let mut dup_lines = Vec::new();
            for (hash, line) in &branch_hashes {
                if let Some(_first_line) = seen.get(hash) {
                    dup_lines.push(*line);
                } else {
                    seen.insert(*hash, *line);
                }
            }

            if !dup_lines.is_empty() {
                let if_line = node.start_position().row as u32 + 1;
                for dup_line in dup_lines {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: dup_line,
                        column: 1,
                        severity: "warning".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "duplicate_elif_branch".to_string(),
                        message: format!(
                            "branch at line {dup_line} has a duplicate body (if-statement at line {if_line})"
                        ),
                        snippet: String::new(),
                    });
                }
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_duplicate_branches(child, source, file_path, pipeline_name, findings);
    }
}

impl GraphPipeline for DuplicateCodePipeline {
    fn name(&self) -> &str {
        "duplicate_code"
    }

    fn description(&self) -> &str {
        "Detects duplicate function bodies and duplicate if/elif branches"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut findings = Vec::new();
        findings.extend(self.check_duplicate_function_bodies(tree, source, file_path));
        findings.extend(self.check_duplicate_elif_branches(tree, source, file_path));
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        use crate::audit::pipeline::GraphPipelineContext;
        use crate::graph::CodeGraph;
        use std::collections::HashMap;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DuplicateCodePipeline::new().unwrap();
        let graph = CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    // ── duplicate_function_body ──

    #[test]
    fn detects_duplicate_function_bodies() {
        let src = "\
def do_a(x):
    y = x + 1
    z = y * 2
    w = z + 3
    v = w - 1
    return v

def do_b(x):
    y = x + 1
    z = y * 2
    w = z + 3
    v = w - 1
    return v
";
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert_eq!(dups.len(), 2); // Both functions reported
    }

    #[test]
    fn clean_unique_function_bodies() {
        let src = "\
def do_a(x):
    y = x + 1
    z = y * 2
    w = z + 3
    v = w - 1
    return v

def do_b(x):
    y = x + 1
    z = y * 3
    w = z - 3
    v = w + 1
    return v
";
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_function_body")
            .collect();
        assert!(dups.is_empty());
    }

    // ── duplicate_elif_branch ──

    #[test]
    fn detects_duplicate_elif_branches() {
        let src = "\
def classify(x):
    if x == 1:
        result = 'low'
        return result
    elif x == 2:
        result = 'medium'
        return result
    elif x == 3:
        result = 'low'
        return result
";
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_elif_branch")
            .collect();
        assert!(!dups.is_empty());
    }

    #[test]
    fn clean_unique_branches() {
        let src = "\
def classify(x):
    if x == 1:
        return 'low'
    elif x == 2:
        return 'medium'
    elif x == 3:
        return 'high'
";
        let findings = parse_and_check(src);
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "duplicate_elif_branch")
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
