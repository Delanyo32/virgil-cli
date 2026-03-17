use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::find_unreachable_after;

use super::primitives::{extract_snippet, find_identifier_in_declarator, has_storage_class, node_text};

pub struct DeadCodePipeline;

impl DeadCodePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn check_unused_static_functions(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Walk for function_definition nodes with `static` storage class
        let mut static_fns: Vec<(String, tree_sitter::Node)> = Vec::new();
        collect_static_functions(root, source, &mut static_fns);

        for (name, fn_node) in &static_fns {
            // Count how many times this identifier appears outside its own definition name
            let mut usage_count = 0;
            let name_node_id = fn_node
                .child_by_field_name("declarator")
                .and_then(|d| find_name_node_in_declarator(d))
                .map(|n| n.id());

            count_identifier_usages(root, source, name, name_node_id, &mut usage_count);

            if usage_count == 0 {
                let start = fn_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_static_function".to_string(),
                    message: format!(
                        "static function `{name}` appears unused within this file"
                    ),
                    snippet: extract_snippet(source, *fn_node, 1),
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

        let return_kinds = ["return_statement", "break_statement", "continue_statement"];
        collect_unreachable_in_blocks(root, source, &return_kinds, file_path, self.name(), &mut findings);

        findings
    }
}

/// Recursively collect `function_definition` nodes that have a `static` storage class specifier.
fn collect_static_functions<'a>(
    node: tree_sitter::Node<'a>,
    source: &[u8],
    out: &mut Vec<(String, tree_sitter::Node<'a>)>,
) {
    if node.kind() == "function_definition" {
        if has_storage_class(node, source, "static") {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name) = find_identifier_in_declarator(declarator, source) {
                    out.push((name, node));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_static_functions(child, source, out);
    }
}

/// Find the identifier/field_identifier node within a declarator chain.
fn find_name_node_in_declarator<'a>(
    node: tree_sitter::Node<'a>,
) -> Option<tree_sitter::Node<'a>> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return Some(node);
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return find_name_node_in_declarator(inner);
    }
    // Walk children for qualified_identifier etc.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "field_identifier" {
            return Some(child);
        }
    }
    None
}

/// Count usages of `target_name` identifier across the tree, excluding the node with `exclude_id`.
fn count_identifier_usages(
    node: tree_sitter::Node,
    source: &[u8],
    target_name: &str,
    exclude_id: Option<usize>,
    count: &mut usize,
) {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        if exclude_id.map_or(true, |eid| node.id() != eid) {
            if node_text(node, source) == target_name {
                *count += 1;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_identifier_usages(child, source, target_name, exclude_id, count);
    }
}

/// Walk all compound_statement nodes and find unreachable code after return/break/continue.
fn collect_unreachable_in_blocks(
    node: tree_sitter::Node,
    source: &[u8],
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
                message: "code is unreachable after return/break/continue".to_string(),
                snippet: String::new(),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unreachable_in_blocks(child, source, return_kinds, file_path, pipeline_name, findings);
    }
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused static functions and unreachable code in C++"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_unused_static_functions(tree, source, file_path));
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    // ── unused_static_function ──

    #[test]
    fn detects_unused_static_function() {
        let src = r#"
static int unusedHelper() {
    return 42;
}

int main() {
    return 0;
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_static_function")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("unusedHelper"));
    }

    #[test]
    fn clean_used_static_function() {
        let src = r#"
static int helper() {
    return 42;
}

int main() {
    return helper();
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_static_function")
            .collect();
        assert!(unused.is_empty());
    }

    #[test]
    fn skips_non_static_function() {
        let src = r#"
int unusedButExported() {
    return 42;
}

int main() {
    return 0;
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_static_function")
            .collect();
        assert!(unused.is_empty());
    }

    // ── unreachable_code ──

    #[test]
    fn detects_unreachable_after_return() {
        let src = r#"
int foo() {
    return 1;
    int x = 2;
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
int foo() {
    int x = 1;
    return x;
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
