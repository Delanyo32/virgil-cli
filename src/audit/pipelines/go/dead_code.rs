use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::find_unreachable_after;

use super::primitives::{extract_snippet, node_text};

pub struct DeadCodePipeline;

impl DeadCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_unused_private_functions(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Walk root children looking for function_declaration with lowercase names
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() != "function_declaration" {
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

            // Skip exported functions (uppercase first letter) and init/main
            let first_char = name.chars().next().unwrap();
            if first_char.is_uppercase() || name == "init" || name == "main" {
                continue;
            }

            // Collect identifiers excluding this function's own declaration name
            // We need to check if the name appears anywhere else in the file
            let mut usage_count = 0;
            count_identifier_usages(root, source, name, name_node.id(), &mut usage_count);

            if usage_count == 0 {
                let start = child.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_private_function".to_string(),
                    message: format!(
                        "unexported function `{name}` appears unused within this file"
                    ),
                    snippet: extract_snippet(source, child, 1),
                });
            }
        }

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

        // Collect identifiers from non-import nodes
        let mut non_import_ids: HashSet<String> = HashSet::new();
        let mut root_cursor = root.walk();
        for child in root.children(&mut root_cursor) {
            if child.kind() != "import_declaration" {
                collect_identifiers_into(child, source, &mut non_import_ids);
            }
        }

        // Walk for import_spec nodes (they can be nested inside import_declaration)
        collect_import_specs(root, source, &non_import_ids, file_path, self.name(), &mut findings);

        findings
    }

    fn check_unreachable_code(
        &self,
        tree: &Tree,
        _source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let return_kinds = ["return_statement", "break_statement", "continue_statement"];
        collect_unreachable_in_blocks(root, &return_kinds, file_path, self.name(), &mut findings);

        findings
    }
}

/// Count usages of `target_name` identifier across the tree, excluding the node with `exclude_id`.
fn count_identifier_usages(
    node: tree_sitter::Node,
    source: &[u8],
    target_name: &str,
    exclude_id: usize,
    count: &mut usize,
) {
    if node.kind() == "identifier" && node.id() != exclude_id {
        if node_text(node, source) == target_name {
            *count += 1;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_identifier_usages(child, source, target_name, exclude_id, count);
    }
}

fn collect_identifiers_into(
    node: tree_sitter::Node,
    source: &[u8],
    ids: &mut HashSet<String>,
) {
    let kind = node.kind();
    if kind == "identifier"
        || kind == "field_identifier"
        || kind == "type_identifier"
        || kind == "property_identifier"
    {
        if let Ok(text) = node.utf8_text(source) {
            ids.insert(text.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers_into(child, source, ids);
    }
}

fn collect_import_specs(
    node: tree_sitter::Node,
    source: &[u8],
    non_import_ids: &HashSet<String>,
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "import_spec" {
        // An import_spec can have an optional alias (name field) and a path
        let alias = node.child_by_field_name("name").map(|n| node_text(n, source).to_string());
        let path_node = node.child_by_field_name("path");

        let imported_name = if let Some(ref alias_name) = alias {
            // If alias is "_", skip (blank import for side effects)
            if alias_name == "_" {
                // Don't flag blank imports
                walk_children_for_import_specs(node, source, non_import_ids, file_path, pipeline_name, findings);
                return;
            }
            // If alias is ".", skip (dot import)
            if alias_name == "." {
                walk_children_for_import_specs(node, source, non_import_ids, file_path, pipeline_name, findings);
                return;
            }
            alias_name.clone()
        } else if let Some(path) = path_node {
            // Extract last path segment from the import path, strip quotes
            let path_text = node_text(path, source);
            let path_text = path_text.trim_matches('"');
            path_text
                .rsplit('/')
                .next()
                .unwrap_or(path_text)
                .to_string()
        } else {
            walk_children_for_import_specs(node, source, non_import_ids, file_path, pipeline_name, findings);
            return;
        };

        if !imported_name.is_empty() && !non_import_ids.contains(&imported_name) {
            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "info".to_string(),
                pipeline: pipeline_name.to_string(),
                pattern: "unused_import".to_string(),
                message: format!(
                    "import `{imported_name}` appears unused"
                ),
                snippet: extract_snippet(source, node, 1),
            });
        }
    }

    walk_children_for_import_specs(node, source, non_import_ids, file_path, pipeline_name, findings);
}

fn walk_children_for_import_specs(
    node: tree_sitter::Node,
    source: &[u8],
    non_import_ids: &HashSet<String>,
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_spec" || child.kind() == "import_declaration" || child.kind() == "import_spec_list" {
            collect_import_specs(child, source, non_import_ids, file_path, pipeline_name, findings);
        }
    }
}

fn collect_unreachable_in_blocks(
    node: tree_sitter::Node,
    return_kinds: &[&str],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
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
                message: "code is unreachable after return/break/continue".to_string(),
                snippet: String::new(),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unreachable_in_blocks(child, return_kinds, file_path, pipeline_name, findings);
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused private functions, unused imports, and unreachable code"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_unused_private_functions(tree, source, file_path));
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    // ── unused_private_function ──

    #[test]
    fn detects_unused_private_function() {
        let src = r#"package main

func main() {
    println("hello")
}

func unusedHelper() {
    println("never called")
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("unusedHelper"));
    }

    #[test]
    fn clean_used_private_function() {
        let src = r#"package main

func main() {
    helper()
}

func helper() {
    println("called")
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn skips_exported_function() {
        let src = r#"package main

func ExportedButUnused() {
    println("exported")
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(unused.is_empty());
    }

    // ── unused_import ──

    #[test]
    fn detects_unused_import() {
        let src = r#"package main

import "fmt"

func main() {
    println("hello")
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("fmt"));
    }

    #[test]
    fn clean_used_import() {
        let src = r#"package main

import "fmt"

func main() {
    fmt.Println("hello")
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn skips_blank_import() {
        let src = r#"package main

import _ "net/http/pprof"

func main() {}
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
        let src = r#"package main

func foo() int {
    return 1
    x := 2
    _ = x
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
        let src = r#"package main

func foo() int {
    x := 1
    return x
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
