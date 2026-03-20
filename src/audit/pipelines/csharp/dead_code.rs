use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_all_identifier_occurrences, find_unreachable_after};

use super::primitives::{extract_snippet, has_modifier, node_text};

pub struct DeadCodePipeline;

impl DeadCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_unused_private_methods(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Walk for class_declaration -> declaration_list -> method_declaration
        find_private_methods_iterative(root, source, file_path, self.name(), &mut findings);

        findings
    }

    fn check_unused_imports(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Collect identifiers from non-using-directive nodes
        let mut non_import_ids: HashSet<String> = HashSet::new();
        let mut root_cursor = root.walk();
        for child in root.children(&mut root_cursor) {
            if child.kind() != "using_directive" {
                collect_identifiers_into(child, source, &mut non_import_ids);
            }
        }

        // Walk root children for using_directive nodes
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() != "using_directive" {
                continue;
            }

            // Extract the last identifier from the qualified name
            let imported_name = extract_last_identifier(child, source);
            if imported_name.is_empty() {
                continue;
            }

            if !non_import_ids.contains(&imported_name) {
                let start = child.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_import".to_string(),
                    message: format!("using `{imported_name}` appears unused"),
                    snippet: extract_snippet(source, child, 1),
                });
            }
        }

        findings
    }

    fn check_unreachable_code(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let return_kinds = [
            "return_statement",
            "break_statement",
            "continue_statement",
            "throw_statement",
        ];
        collect_unreachable_in_blocks(
            root,
            source,
            &return_kinds,
            file_path,
            self.name(),
            &mut findings,
        );

        findings
    }
}

fn find_private_methods_iterative(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "class_declaration" {
            let id_counts = count_all_identifier_occurrences(node, source);

            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    if child.kind() != "method_declaration" {
                        continue;
                    }
                    if !has_modifier(child, source, "private") {
                        continue;
                    }
                    let name_node = match child.child_by_field_name("name") {
                        Some(n) => n,
                        None => continue,
                    };
                    let name = node_text(name_node, source);
                    if name.is_empty() {
                        continue;
                    }
                    let total_count = id_counts.get(name).copied().unwrap_or(0);
                    if total_count <= 1 {
                        let start = child.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "unused_private_method".to_string(),
                            message: format!(
                                "private method `{name}` appears unused within this class"
                            ),
                            snippet: extract_snippet(source, child, 1),
                        });
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn collect_identifiers_into(root: tree_sitter::Node, source: &[u8], ids: &mut HashSet<String>) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if (kind == "identifier"
            || kind == "field_identifier"
            || kind == "type_identifier"
            || kind == "property_identifier")
            && let Ok(text) = node.utf8_text(source)
        {
            ids.insert(text.to_string());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Extract the last identifier from a using_directive node (stack-based).
fn extract_last_identifier(using_node: tree_sitter::Node, source: &[u8]) -> String {
    let mut last_id = String::new();
    let mut stack = vec![using_node];
    while let Some(node) = stack.pop() {
        if node.kind() == "identifier"
            && let Ok(text) = node.utf8_text(source)
        {
            last_id = text.to_string();
        }
        let mut cursor = node.walk();
        // Push in reverse so left-to-right order means last identifier wins
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    last_id
}

fn collect_unreachable_in_blocks(
    root: tree_sitter::Node,
    _source: &[u8],
    return_kinds: &[&str],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "block" {
            let unreachable = find_unreachable_after(node, return_kinds);
            for (line, col) in unreachable {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line,
                    column: col,
                    severity: "warning".to_string(),
                    pipeline: pipeline_name.to_string(),
                    pattern: "unreachable_code".to_string(),
                    message: "code is unreachable after return/break/continue/throw".to_string(),
                    snippet: String::new(),
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused private methods, unused imports, and unreachable code in C#"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_unused_private_methods(tree, source, file_path));
        findings.extend(self.check_unused_imports(tree, source, file_path));
        findings.extend(self.check_unreachable_code(tree, source, file_path));
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
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    // ── unused_private_method ──

    #[test]
    fn detects_unused_private_method() {
        let src = r#"
class Foo {
    public void Main() {
        DoWork();
    }

    private void DoWork() { }

    private void NeverCalled() { }
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_method")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("NeverCalled"));
    }

    #[test]
    fn clean_used_private_method() {
        let src = r#"
class Foo {
    public void Main() {
        Helper();
    }

    private void Helper() { }
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_method")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn skips_non_private_methods() {
        let src = r#"
class Foo {
    public void PublicUnused() { }
    internal void InternalUnused() { }
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_method")
            .collect();
        assert!(unused.is_empty());
    }

    // ── unused_import ──

    #[test]
    fn detects_unused_import() {
        let src = r#"
using System;
using System.Collections.Generic;

class Foo {
    public void Main() {
        Console.WriteLine("hello");
    }
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert!(unused.len() >= 1, "should detect at least 1 unused import");
        assert!(unused.iter().any(|f| f.message.contains("Generic")));
    }

    #[test]
    fn clean_used_import() {
        let src = r#"
using System;

class Foo {
    public void Main() {
        System.Console.WriteLine("hello");
    }
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert!(unused.is_empty());
    }

    // ── unreachable_code ──

    #[test]
    fn detects_unreachable_after_return() {
        let src = r#"
class Foo {
    public int Bar() {
        return 1;
        int x = 2;
    }
}
"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(!unreachable.is_empty());
    }

    #[test]
    fn detects_unreachable_after_throw() {
        let src = r#"
class Foo {
    public void Bar() {
        throw new Exception("err");
        int x = 2;
    }
}
"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(!unreachable.is_empty());
    }

    #[test]
    fn clean_no_unreachable() {
        let src = r#"
class Foo {
    public int Bar() {
        int x = 1;
        return x;
    }
}
"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(unreachable.is_empty());
    }

    // ── metadata ──

    #[test]
    fn metadata_check() {
        let pipeline = DeadCodePipeline::new().unwrap();
        assert_eq!(pipeline.name(), "dead_code");
        assert!(!pipeline.description().is_empty());
    }
}
