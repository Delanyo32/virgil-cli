use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{count_all_identifier_occurrences, find_unreachable_after};

use super::primitives::{extract_snippet, find_identifier_in_declarator, has_storage_class};

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

        // Build identifier count map once for the entire file — O(n) instead of O(n*m).
        let id_counts = count_all_identifier_occurrences(root, source);

        for (name, fn_node) in &static_fns {
            // The declaration itself counts as 1. If total <= 1, the function is unused.
            let total_count = id_counts.get(name.as_str()).copied().unwrap_or(0);
            if total_count <= 1 {
                let start = fn_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unused_static_function".to_string(),
                    message: format!("static function `{name}` appears unused within this file"),
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

/// Collect `function_definition` nodes that have a `static` storage class specifier (stack-based).
fn collect_static_functions<'a>(
    root: tree_sitter::Node<'a>,
    source: &[u8],
    out: &mut Vec<(String, tree_sitter::Node<'a>)>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_definition"
            && has_storage_class(node, source, "static")
                && let Some(declarator) = node.child_by_field_name("declarator")
                    && let Some(name) = find_identifier_in_declarator(declarator, source) {
                        out.push((name, node));
                    }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Walk all compound_statement nodes and find unreachable code (stack-based).
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
            stack.push(child);
        }
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
