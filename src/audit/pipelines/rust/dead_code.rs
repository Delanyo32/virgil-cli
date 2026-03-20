use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::Tree;

use super::primitives::extract_snippet;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::find_unreachable_after;

pub struct DeadCodePipeline;

impl DeadCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused private functions, unused imports, and unreachable code in Rust"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── unused_private_function ────────────────────────────────────
        let mut functions: Vec<(String, tree_sitter::Node)> = Vec::new();
        collect_function_items(root, source, &mut functions);

        // Only consider private functions (no visibility_modifier child)
        let private_fns: Vec<&(String, tree_sitter::Node)> = functions
            .iter()
            .filter(|(_, node)| !has_visibility_modifier(*node))
            .collect();

        // Collect identifiers that appear in "usage" positions — everything
        // except the name field of function_item definitions.
        let usage_ids = collect_usage_identifiers(root, source);

        for (name, node) in &private_fns {
            if !usage_ids.contains(name.as_str()) {
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_private_function".to_string(),
                    message: format!("private function `{name}` appears unused in this file"),
                    snippet: extract_snippet(source, *node, 3),
                });
            }
        }

        // ── unused_imports ─────────────────────────────────────────────
        let mut use_decls: Vec<(String, tree_sitter::Node)> = Vec::new();
        collect_use_declarations(root, source, &mut use_decls);

        // Collect identifiers excluding use_declaration subtrees
        let non_import_ids = collect_identifiers_excluding(root, source, "use_declaration");

        for (imported_name, node) in &use_decls {
            if !non_import_ids.contains(imported_name.as_str()) {
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_import".to_string(),
                    message: format!("import `{imported_name}` appears unused in this file"),
                    snippet: extract_snippet(source, *node, 1),
                });
            }
        }

        // ── unreachable_code ───────────────────────────────────────────
        let return_kinds = &[
            "return_expression",
            "break_expression",
            "continue_expression",
        ];
        collect_unreachable_findings(
            root,
            source,
            file_path,
            self.name(),
            return_kinds,
            &mut findings,
        );

        findings
    }
}

/// Collect all `function_item` nodes with their name text (stack-based iteration).
fn collect_function_items<'a>(
    root: tree_sitter::Node<'a>,
    source: &[u8],
    out: &mut Vec<(String, tree_sitter::Node<'a>)>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_item"
            && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(name) = name_node.utf8_text(source)
        {
            out.push((name.to_string(), node));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Check if a node has a `visibility_modifier` child (making it public).
fn has_visibility_modifier(node: tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return true;
        }
    }
    false
}

/// Collect identifiers that represent "usages" — all identifiers in the file
/// EXCEPT those that are the `name` field of a `function_item`.
fn collect_usage_identifiers(root: tree_sitter::Node, source: &[u8]) -> HashSet<String> {
    let mut ids = HashSet::new();
    collect_usage_ids_recursive(root, source, false, &mut ids);
    ids
}

fn collect_usage_ids_recursive(
    root: tree_sitter::Node,
    source: &[u8],
    _skip_this_id: bool,
    ids: &mut HashSet<String>,
) {
    // Stack holds (node, skip_this_id) pairs
    let mut stack: Vec<(tree_sitter::Node, bool)> = vec![(root, _skip_this_id)];
    while let Some((node, skip_this_id)) = stack.pop() {
        let kind = node.kind();

        // If we're at a function_item, push children but mark the name child
        // so its identifier is skipped.
        if kind == "function_item" {
            let name_id = node.child_by_field_name("name").map(|n| n.id());
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let is_fn_name = name_id.map(|id| id == child.id()).unwrap_or(false);
                stack.push((child, is_fn_name));
            }
            continue;
        }

        // Collect identifiers (unless this node is the fn name position)
        if !skip_this_id
            && (kind == "identifier" || kind == "field_identifier" || kind == "type_identifier")
            && let Ok(text) = node.utf8_text(source)
        {
            ids.insert(text.to_string());
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push((child, false));
        }
    }
}

/// Extract the last segment of a use path (the imported name).
fn extract_last_use_segment(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "use_as_clause" => {
                // `use foo::bar as baz;` — alias is the imported name
                if let Some(alias) = child.child_by_field_name("alias") {
                    return alias.utf8_text(source).ok().map(|s| s.to_string());
                }
            }
            "scoped_identifier" => {
                // Last segment of path (e.g., HashMap in std::collections::HashMap)
                if let Some(name) = child.child_by_field_name("name") {
                    return name.utf8_text(source).ok().map(|s| s.to_string());
                }
            }
            "identifier" => {
                return child.utf8_text(source).ok().map(|s| s.to_string());
            }
            _ => {}
        }
    }
    None
}

/// Collect use_declaration nodes with the imported name (last path segment).
fn collect_use_declarations<'a>(
    root: tree_sitter::Node<'a>,
    source: &[u8],
    out: &mut Vec<(String, tree_sitter::Node<'a>)>,
) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "use_declaration"
            && let Some(name) = extract_last_use_segment(child, source)
        {
            // Skip wildcard imports
            if name != "*" {
                out.push((name, child));
            }
        }
    }
}

/// Collect all identifiers in the tree, excluding subtrees of `exclude_kind` nodes.
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
    root: tree_sitter::Node,
    source: &[u8],
    exclude_kind: &str,
    ids: &mut HashSet<String>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == exclude_kind {
            continue;
        }
        let kind = node.kind();
        if (kind == "identifier" || kind == "type_identifier" || kind == "field_identifier")
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

/// Walk all block nodes and find unreachable code after return/break/continue (stack-based iteration).
fn collect_unreachable_findings(
    root: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    return_kinds: &[&str],
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
                    message: "code after return/break/continue is unreachable".to_string(),
                    snippet: extract_snippet(source, node, 3),
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_unused_private_function() {
        let src = r#"
fn unused_helper() {
    println!("never called");
}

fn main() {
    println!("hello");
}
"#;
        let findings = parse_and_check(src);
        let unused_fn: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(
            unused_fn
                .iter()
                .any(|f| f.message.contains("unused_helper")),
            "should flag unused_helper as unused"
        );
    }

    #[test]
    fn skips_used_private_function() {
        let src = r#"
fn helper() -> i32 {
    42
}

fn main() {
    let x = helper();
}
"#;
        let findings = parse_and_check(src);
        let unused_fn: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(
            !unused_fn.iter().any(|f| f.message.contains("helper")),
            "should not flag helper as unused since it's called in main"
        );
    }

    #[test]
    fn skips_public_function() {
        let src = r#"
pub fn unused_but_public() {
    println!("exported");
}

fn main() {}
"#;
        let findings = parse_and_check(src);
        let unused_fn: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(
            !unused_fn
                .iter()
                .any(|f| f.message.contains("unused_but_public")),
            "should not flag pub functions"
        );
    }

    #[test]
    fn detects_unused_import() {
        let src = r#"
use std::collections::HashMap;

fn main() {
    let x = 1;
}
"#;
        let findings = parse_and_check(src);
        let unused_imports: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert!(
            unused_imports.iter().any(|f| f.message.contains("HashMap")),
            "should flag unused HashMap import"
        );
    }

    #[test]
    fn skips_used_import() {
        let src = r#"
use std::collections::HashMap;

fn main() {
    let map: HashMap<String, i32> = HashMap::new();
}
"#;
        let findings = parse_and_check(src);
        let unused_imports: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_import")
            .collect();
        assert!(
            !unused_imports.iter().any(|f| f.message.contains("HashMap")),
            "should not flag used HashMap import"
        );
    }

    #[test]
    fn detects_unreachable_code() {
        let src = r#"
fn example() {
    return;
    let x = 1;
}
"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(!unreachable.is_empty(), "should detect code after return");
    }

    #[test]
    fn no_unreachable_in_clean_code() {
        let src = r#"
fn example() -> i32 {
    let x = 1;
    let y = 2;
    x + y
}
"#;
        let findings = parse_and_check(src);
        let unreachable: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unreachable_code")
            .collect();
        assert!(
            unreachable.is_empty(),
            "clean code should have no unreachable findings"
        );
    }

    #[test]
    fn correct_metadata() {
        let src = r#"
fn unused_fn() {}
fn main() {}
"#;
        let findings = parse_and_check(src);
        let unused_fn: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_private_function")
            .collect();
        assert!(!unused_fn.is_empty());
        let f = &unused_fn[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "dead_code");
        assert_eq!(f.severity, "info");
    }
}
