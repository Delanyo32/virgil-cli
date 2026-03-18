use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{collect_identifiers, find_unreachable_after};
use crate::audit::primitives::extract_snippet;
use crate::language::Language;

pub struct DeadCodePipeline {
    _language: Language,
}

impl DeadCodePipeline {
    pub fn new(language: Language) -> Result<Self> {
        // Validate the language can produce a tree-sitter language
        let _ts_lang = language.tree_sitter_language();
        Ok(Self {
            _language: language,
        })
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused imports and unreachable code after return/break/continue/throw"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── unused_imports ──────────────────────────────────────────
        // Collect all identifiers from non-import nodes, including type-position identifiers
        let mut used_ids: HashSet<String> = HashSet::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() != "import_statement" {
                let ids = collect_identifiers(child, source);
                used_ids.extend(ids);
            }
        }
        // Also collect type identifiers from type annotations, type aliases, and interfaces
        // so that type-only imports used in type positions aren't flagged as unused
        collect_type_identifiers(root, source, &mut used_ids);

        // Walk import statements and extract imported names
        let mut import_cursor = root.walk();
        for child in root.children(&mut import_cursor) {
            if child.kind() != "import_statement" {
                continue;
            }
            let imported_names = extract_import_names(child, source);
            for (name, name_line, name_col) in imported_names {
                if !used_ids.contains(&name) {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: name_line,
                        column: name_col,
                        severity: "info".to_string(),
                        pipeline: "dead_code".to_string(),
                        pattern: "unused_imports".to_string(),
                        message: format!(
                            "Import `{}` is not used in this file",
                            name
                        ),
                        snippet: extract_snippet(source, child, 1),
                    });
                }
            }
        }

        // ── unreachable_code ────────────────────────────────────────
        let return_kinds = [
            "return_statement",
            "break_statement",
            "continue_statement",
            "throw_statement",
        ];
        find_unreachable_blocks(root, &return_kinds, file_path, source, &mut findings);

        findings
    }
}

/// Recursively find all statement_block nodes and check for unreachable code.
fn find_unreachable_blocks(
    node: tree_sitter::Node,
    return_kinds: &[&str],
    file_path: &str,
    source: &[u8],
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "statement_block" {
        let positions = find_unreachable_after(node, return_kinds);
        for (line, col) in positions {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line,
                column: col,
                severity: "warning".to_string(),
                pipeline: "dead_code".to_string(),
                pattern: "unreachable_code".to_string(),
                message: "Code is unreachable after return/break/continue/throw".to_string(),
                snippet: extract_snippet(source, node, 3),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_unreachable_blocks(child, return_kinds, file_path, source, findings);
    }
}

/// Recursively collect identifiers from type_annotation, type_alias_declaration,
/// and interface_declaration nodes throughout the tree. This ensures type-only imports
/// used in type positions are recognized as used.
fn collect_type_identifiers(
    node: tree_sitter::Node,
    source: &[u8],
    ids: &mut HashSet<String>,
) {
    const TYPE_NODE_KINDS: &[&str] = &[
        "type_annotation",
        "type_alias_declaration",
        "interface_declaration",
    ];

    if TYPE_NODE_KINDS.contains(&node.kind()) {
        let type_ids = collect_identifiers(node, source);
        ids.extend(type_ids);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_identifiers(child, source, ids);
    }
}

/// Extract imported names from an import_statement node.
/// Handles: `import { a, b } from '...'`, `import x from '...'`,
/// `import * as ns from '...'`, `import { a as b } from '...'`
fn extract_import_names(import_node: tree_sitter::Node, source: &[u8]) -> Vec<(String, u32, u32)> {
    let mut names = Vec::new();
    extract_import_names_recursive(import_node, source, &mut names);
    names
}

fn extract_import_names_recursive(
    node: tree_sitter::Node,
    source: &[u8],
    names: &mut Vec<(String, u32, u32)>,
) {
    match node.kind() {
        // `import x from '...'` — default import
        "import_clause" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_import_names_recursive(child, source, names);
            }
        }
        // `import { a, b as c } from '...'`
        "named_imports" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "import_specifier" {
                    // If aliased: `import { a as b }` — the local name is `b` (alias field)
                    let local_name_node = child
                        .child_by_field_name("alias")
                        .or_else(|| child.child_by_field_name("name"));
                    if let Some(name_node) = local_name_node {
                        let text = name_node.utf8_text(source).unwrap_or("");
                        if !text.is_empty() {
                            let pos = name_node.start_position();
                            names.push((
                                text.to_string(),
                                pos.row as u32 + 1,
                                pos.column as u32 + 1,
                            ));
                        }
                    }
                }
            }
        }
        // `import * as ns from '...'`
        "namespace_import" => {
            // The identifier after `as`
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "identifier" {
                    let text = child.utf8_text(source).unwrap_or("");
                    if !text.is_empty() {
                        let pos = child.start_position();
                        names.push((
                            text.to_string(),
                            pos.row as u32 + 1,
                            pos.column as u32 + 1,
                        ));
                    }
                }
            }
        }
        // Default import identifier
        "identifier" => {
            // Only if parent is import_clause (default import)
            if let Some(parent) = node.parent() {
                if parent.kind() == "import_clause" {
                    let text = node.utf8_text(source).unwrap_or("");
                    if !text.is_empty() {
                        let pos = node.start_position();
                        names.push((
                            text.to_string(),
                            pos.row as u32 + 1,
                            pos.column as u32 + 1,
                        ));
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_import_names_recursive(child, source, names);
            }
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
        let pipeline = DeadCodePipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_unused_named_import() {
        let source = r#"
import { foo, bar } from './module';
console.log(foo);
"#;
        let findings = parse_and_check(source);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_imports")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("bar"));
    }

    #[test]
    fn detects_unused_default_import() {
        let source = r#"
import React from 'react';
import { useState } from 'react';
const state = useState(0);
"#;
        let findings = parse_and_check(source);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_imports")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("React"));
    }

    #[test]
    fn no_finding_when_all_imports_used() {
        let source = r#"
import { foo, bar } from './module';
console.log(foo, bar);
"#;
        let findings = parse_and_check(source);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_imports")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn detects_unreachable_code_after_return() {
        let source = r#"
function example() {
    return 42;
    console.log("unreachable");
}
"#;
        let findings = parse_and_check(source);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert_eq!(unreachable.len(), 1);
        assert_eq!(unreachable[0].severity, "warning");
    }

    #[test]
    fn detects_unreachable_code_after_throw() {
        let source = r#"
function example() {
    throw new Error("fail");
    const x = 1;
}
"#;
        let findings = parse_and_check(source);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert_eq!(unreachable.len(), 1);
    }

    #[test]
    fn no_finding_when_no_unreachable_code() {
        let source = r#"
function example(x: number): number {
    if (x > 0) {
        return x;
    }
    return 0;
}
"#;
        let findings = parse_and_check(source);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(unreachable.is_empty());
    }

    #[test]
    fn detects_unused_namespace_import() {
        let source = r#"
import * as utils from './utils';
const x = 1;
"#;
        let findings = parse_and_check(source);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_imports")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("utils"));
    }

    #[test]
    fn metadata_is_correct() {
        let pipeline = DeadCodePipeline::new(Language::TypeScript).unwrap();
        assert_eq!(pipeline.name(), "dead_code");
        assert!(!pipeline.description().is_empty());
    }
}
