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
}

impl Pipeline for DeadCodePipeline {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn description(&self) -> &str {
        "Detects unused static functions and unreachable code in C"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // ── unused_static_function ───────────────────────────────────
        // In C, `static` storage class = file-scoped (private). Walk all
        // function_definition nodes, check for `static`, then see if the
        // function name appears anywhere else in the file.

        let mut static_fns: Vec<(String, tree_sitter::Node)> = Vec::new();
        collect_static_functions(root, source, &mut static_fns);

        for (name, node) in &static_fns {
            // Count usages of this name excluding the definition's own declarator
            let mut usage_count = 0;
            let declarator_node = node.child_by_field_name("declarator");
            let exclude_id = declarator_node.map(|d| d.id());
            count_identifier_usages(root, source, name, exclude_id, &mut usage_count);

            if usage_count == 0 {
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_static_function".to_string(),
                    message: format!(
                        "static function `{name}` appears unused in this file"
                    ),
                    snippet: extract_snippet(source, *node, 3),
                });
            }
        }

        // ── unreachable_code ─────────────────────────────────────────
        let return_kinds = [
            "return_statement",
            "break_statement",
            "continue_statement",
        ];
        collect_unreachable_findings(
            root,
            source,
            file_path,
            self.name(),
            &return_kinds,
            &mut findings,
        );

        findings
    }
}

/// Recursively collect all `function_definition` nodes that have `static` storage class.
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

/// Count usages of `target_name` identifier across the tree, excluding nodes
/// that are descendants of the node with `exclude_subtree_id`.
fn count_identifier_usages(
    node: tree_sitter::Node,
    source: &[u8],
    target_name: &str,
    exclude_subtree_id: Option<usize>,
    count: &mut usize,
) {
    // Skip the entire declarator subtree of the definition itself
    if let Some(excl_id) = exclude_subtree_id {
        if node.id() == excl_id {
            return;
        }
    }
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        if node_text(node, source) == target_name {
            *count += 1;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_identifier_usages(child, source, target_name, exclude_subtree_id, count);
    }
}

/// Walk all `compound_statement` nodes and find unreachable code after
/// return/break/continue.
fn collect_unreachable_findings(
    node: tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    pipeline_name: &str,
    return_kinds: &[&str],
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
                message: "code after return/break/continue is unreachable".to_string(),
                snippet: String::new(),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unreachable_findings(child, source, file_path, pipeline_name, return_kinds, findings);
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
        let pipeline = DeadCodePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    // ── unused_static_function ──

    #[test]
    fn detects_unused_static_function() {
        let src = r#"
static int unused_helper(void) {
    return 42;
}

int main(void) {
    return 0;
}
"#;
        let findings = parse_and_check(src);
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unused_static_function")
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("unused_helper"));
    }

    #[test]
    fn skips_used_static_function() {
        let src = r#"
static int helper(void) {
    return 42;
}

int main(void) {
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
int unused_but_not_static(void) {
    return 42;
}

int main(void) {
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
int foo(void) {
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
    fn no_unreachable_in_clean_code() {
        let src = r#"
int foo(void) {
    int x = 1;
    int y = 2;
    return x + y;
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
        assert_eq!(pipeline.description(), "Detects unused static functions and unreachable code in C");
    }
}
