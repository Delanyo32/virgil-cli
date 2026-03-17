use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{collect_identifiers, find_unreachable_after};

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

        // Collect all identifiers in the file for usage checking
        let all_identifiers = collect_identifiers(root, source);

        // Walk the tree looking for method_declaration nodes inside class_body
        collect_unused_private_methods(
            root,
            source,
            &all_identifiers,
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

        // Collect all identifiers outside of import_declaration subtrees
        let non_import_ids = collect_identifiers_excluding(root, source, "import_declaration");

        // Walk root children for import_declaration nodes
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() != "import_declaration" {
                continue;
            }

            // Extract the last segment of the import path
            if let Some(imported_name) = extract_import_last_segment(child, source) {
                if imported_name == "*" {
                    continue; // skip wildcard imports
                }
                if !non_import_ids.contains(imported_name.as_str()) {
                    let start = child.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unused_import".to_string(),
                        message: format!(
                            "import `{imported_name}` appears unused in this file"
                        ),
                        snippet: extract_snippet(source, child, 1),
                    });
                }
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

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused private methods, unused imports, and unreachable code in Java"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_unused_private_methods(tree, source, file_path));
        findings.extend(self.check_unused_imports(tree, source, file_path));
        findings.extend(self.check_unreachable_code(tree, source, file_path));
        findings
    }
}

/// Walk the tree looking for private method_declaration nodes inside class_body.
/// If the method name does not appear elsewhere in the file (excluding its own
/// definition name), flag as unused.
fn collect_unused_private_methods(
    node: tree_sitter::Node,
    source: &[u8],
    _all_identifiers: &HashSet<String>,
    file_path: &str,
    pipeline_name: &str,
    findings: &mut Vec<AuditFinding>,
) {
    if node.kind() == "class_body" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "method_declaration" {
                continue;
            }

            // Check if the method has "private" modifier
            if !has_modifier(child, source, "private") {
                continue;
            }

            let name_node = match child.child_by_field_name("name") {
                Some(n) => n,
                None => continue,
            };

            let method_name = node_text(name_node, source);
            if method_name.is_empty() {
                continue;
            }

            // Count usages of this method name in the file, excluding
            // the definition name node itself
            let root = find_root(node);
            let mut usage_count = 0;
            count_identifier_usages(root, source, method_name, name_node.id(), &mut usage_count);

            if usage_count == 0 {
                let start = child.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: pipeline_name.to_string(),
                    pattern: "unused_private_method".to_string(),
                    message: format!(
                        "private method `{method_name}` appears unused in this file"
                    ),
                    snippet: extract_snippet(source, child, 3),
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unused_private_methods(child, source, _all_identifiers, file_path, pipeline_name, findings);
    }
}

/// Navigate to the root node from any node.
fn find_root(node: tree_sitter::Node) -> tree_sitter::Node {
    let mut current = node;
    while let Some(parent) = current.parent() {
        current = parent;
    }
    current
}

/// Count usages of `target_name` as an identifier, excluding the node with `exclude_id`.
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

/// Extract the last identifier segment from a Java import_declaration.
/// e.g. `import java.util.HashMap;` -> "HashMap"
/// e.g. `import java.util.*;` -> "*"
fn extract_import_last_segment(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // In tree-sitter-java, import_declaration contains a scoped_identifier
    // or an asterisk pattern. Walk named children to find it.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" => {
                // The last named child or the "name" field is the imported name
                if let Some(name) = child.child_by_field_name("name") {
                    return name.utf8_text(source).ok().map(|s| s.to_string());
                }
            }
            "identifier" => {
                return child.utf8_text(source).ok().map(|s| s.to_string());
            }
            "asterisk" => {
                return Some("*".to_string());
            }
            _ => {}
        }
    }
    None
}

/// Collect all identifiers in the tree, skipping subtrees of `exclude_kind`.
fn collect_identifiers_excluding(
    root: tree_sitter::Node,
    source: &[u8],
    exclude_kind: &str,
) -> HashSet<String> {
    let mut ids = HashSet::new();
    collect_ids_excluding_recursive(root, source, exclude_kind, &mut ids);
    ids
}

fn collect_ids_excluding_recursive(
    node: tree_sitter::Node,
    source: &[u8],
    exclude_kind: &str,
    ids: &mut HashSet<String>,
) {
    if node.kind() == exclude_kind {
        return;
    }
    let kind = node.kind();
    if kind == "identifier" || kind == "type_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            ids.insert(text.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ids_excluding_recursive(child, source, exclude_kind, ids);
    }
}

/// Walk all block nodes and find unreachable code after return/break/continue/throw.
fn collect_unreachable_in_blocks(
    node: tree_sitter::Node,
    source: &[u8],
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
                message: "code after return/break/continue/throw is unreachable".to_string(),
                snippet: String::new(),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unreachable_in_blocks(child, source, return_kinds, file_path, pipeline_name, findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    // ── unused_private_method ──

    #[test]
    fn detects_unused_private_method() {
        let src = r#"
class Foo {
    private void unusedHelper() {
        System.out.println("never called");
    }

    public void main() {
        System.out.println("hello");
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
        let src = r#"
class Foo {
    private int helper() {
        return 42;
    }

    public void main() {
        int x = helper();
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
        let src = r#"
class Foo {
    public void unusedButPublic() {
        System.out.println("exported");
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
    fn detects_unused_import() {
        let src = r#"
import java.util.HashMap;

class Foo {
    public void main() {
        int x = 1;
    }
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("HashMap"));
    }

    #[test]
    fn clean_used_import() {
        let src = r#"
import java.util.HashMap;

class Foo {
    public void main() {
        HashMap<String, Integer> map = new HashMap<>();
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
    public int example() {
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
    public void example() {
        throw new RuntimeException("fail");
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
    public int example() {
        int x = 1;
        int y = 2;
        return x + y;
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
