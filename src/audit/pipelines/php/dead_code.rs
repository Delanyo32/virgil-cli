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

    fn check_unused_private_methods(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Walk for class_declaration -> declaration_list -> method_declaration
        collect_unused_private_methods(
            root,
            source,
            file_path,
            self.name(),
            &mut findings,
        );

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
        collect_non_import_identifiers(root, source, &mut non_import_ids);

        // Walk for use_declaration, include_expression, require_expression
        collect_unused_imports(root, source, &non_import_ids, file_path, self.name(), &mut findings);

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
            "throw_expression",
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

fn collect_unused_private_methods(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "class_declaration" {
        if let Some(body) = node.child_by_field_name("body") {
            let mut body_cursor = body.walk();
            for child in body.named_children(&mut body_cursor) {
                if child.kind() != "method_declaration" {
                    continue;
                }

                // Check for visibility_modifier child with text "private"
                let is_private = has_visibility(child, source, "private");
                if !is_private {
                    continue;
                }

                let name = match child.child_by_field_name("name") {
                    Some(n) => node_text(n, source),
                    None => continue,
                };

                if name.is_empty() || name == "__construct" || name == "__destruct" {
                    continue;
                }

                // Check if the name appears in identifiers elsewhere
                // The all_identifiers set collects "name" kind nodes, which includes
                // method names in call sites like $this->methodName()
                // We need to count usages excluding the declaration itself
                let name_node = child.child_by_field_name("name").unwrap();
                let mut usage_count = 0;
                count_name_usages(
                    node, // Search within the class
                    source,
                    name,
                    name_node.id(),
                    &mut usage_count,
                );

                if usage_count == 0 {
                    // Additionally check if the method name appears in any string
                    // literal in the class body (covers `call_user_func('methodName')`
                    // and similar dynamic dispatch patterns).
                    let referenced_in_string = string_contains_name(body, source, name);
                    if referenced_in_string {
                        continue;
                    }

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
        collect_unused_private_methods(child, source, file_path, pipeline_name, findings);
    }
}

fn has_visibility(method: tree_sitter::Node, source: &[u8], target: &str) -> bool {
    let mut cursor = method.walk();
    for child in method.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(child, source);
            if text == target {
                return true;
            }
        }
    }
    false
}

/// Check if any `string` or `encapsed_string` node within `root` contains `target_name`.
/// This covers dynamic dispatch patterns like `call_user_func('methodName')`.
fn string_contains_name(root: tree_sitter::Node, source: &[u8], target_name: &str) -> bool {
    let mut found = false;
    string_contains_name_recursive(root, source, target_name, &mut found);
    found
}

fn string_contains_name_recursive(
    node: tree_sitter::Node,
    source: &[u8],
    target_name: &str,
    found: &mut bool,
) {
    if *found {
        return;
    }
    let kind = node.kind();
    if kind == "string" || kind == "encapsed_string" {
        if let Ok(text) = node.utf8_text(source) {
            if text.contains(target_name) {
                *found = true;
                return;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        string_contains_name_recursive(child, source, target_name, found);
    }
}

/// Count usages of a name (via `name` kind nodes) across the tree, excluding the node with `exclude_id`.
fn count_name_usages(
    node: tree_sitter::Node,
    source: &[u8],
    target_name: &str,
    exclude_id: usize,
    count: &mut usize,
) {
    if node.kind() == "name" && node.id() != exclude_id {
        if node_text(node, source) == target_name {
            *count += 1;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_name_usages(child, source, target_name, exclude_id, count);
    }
}

fn collect_non_import_identifiers(
    node: tree_sitter::Node,
    source: &[u8],
    ids: &mut HashSet<String>,
) {
    let kind = node.kind();

    // Skip import-like nodes
    if kind == "namespace_use_declaration"
        || kind == "include_expression"
        || kind == "include_once_expression"
        || kind == "require_expression"
        || kind == "require_once_expression"
    {
        return;
    }

    if kind == "name" || kind == "qualified_name" {
        if let Ok(text) = node.utf8_text(source) {
            ids.insert(text.to_string());
            // Also insert the last segment for qualified names
            if let Some(last) = text.rsplit('\\').next() {
                ids.insert(last.to_string());
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_non_import_identifiers(child, source, ids);
    }
}

fn collect_unused_imports(
    node: tree_sitter::Node,
    source: &[u8],
    non_import_ids: &HashSet<String>,
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    let kind = node.kind();

    if kind == "namespace_use_declaration" {
        // PHP `use` statement: use App\Models\User;
        // Walk children for namespace_use_clause which contains qualified_name
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "namespace_use_clause" {
                // Get the qualified name and extract last segment
                let full_text = node_text(child, source).trim().to_string();
                let last_segment = full_text
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&full_text)
                    .to_string();

                // Check if alias is present
                let alias = child.child_by_field_name("alias");
                let imported_name = if let Some(alias_node) = alias {
                    node_text(alias_node, source).to_string()
                } else {
                    last_segment
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
                        message: format!("import `{imported_name}` appears unused"),
                        snippet: extract_snippet(source, node, 1),
                    });
                }
            }
        }
        return;
    }

    if kind == "include_expression"
        || kind == "include_once_expression"
        || kind == "require_expression"
        || kind == "require_once_expression"
    {
        // For include/require, we typically don't flag as unused since they have side effects
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unused_imports(child, source, non_import_ids, file_path, pipeline_name, findings);
    }
}

fn collect_unreachable_in_blocks(
    node: tree_sitter::Node,
    _source: &[u8],
    return_kinds: &[&str],
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "compound_statement" {
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
        collect_unreachable_in_blocks(child, _source, return_kinds, file_path, pipeline_name, findings);
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused private methods, unused imports, and unreachable code in PHP"
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    // ── unused_private_method ──

    #[test]
    fn detects_unused_private_method() {
        let src = r#"<?php
class MyService {
    public function doWork() {
        return 42;
    }

    private function unusedHelper() {
        return "never called";
    }
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_method")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("unusedHelper"));
    }

    #[test]
    fn clean_used_private_method() {
        let src = r#"<?php
class MyService {
    public function doWork() {
        return $this->helper();
    }

    private function helper() {
        return 42;
    }
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
    fn skips_public_method() {
        let src = r#"<?php
class MyService {
    public function publicUnused() {
        return 1;
    }
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
    fn detects_unused_use_import() {
        let src = r#"<?php
use App\Models\User;

function main() {
    return 42;
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("User"));
    }

    #[test]
    fn clean_used_import() {
        let src = r#"<?php
use App\Models\User;

function main() {
    $user = new User();
    return $user;
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
        let src = r#"<?php
function foo() {
    return 1;
    $x = 2;
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
        let src = r#"<?php
function foo() {
    $x = 1;
    return $x;
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
