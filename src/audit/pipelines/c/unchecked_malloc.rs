use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, extract_snippet, find_capture_index,
    find_identifier_in_declarator, node_text,
};

const ALLOC_FUNCTIONS: &[&str] = &["malloc", "calloc", "realloc"];

pub struct UncheckedMallocPipeline {
    fn_def_query: Arc<Query>,
}

impl UncheckedMallocPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_def_query: compile_function_definition_query()?,
        })
    }

    fn find_alloc_calls_in_body<'a>(
        body: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> Vec<(tree_sitter::Node<'a>, String)> {
        // Returns (call_node, assigned_variable_name) pairs
        let mut results = Vec::new();
        Self::walk_for_allocs(body, source, &mut results);
        results
    }

    fn walk_for_allocs<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        results: &mut Vec<(tree_sitter::Node<'a>, String)>,
    ) {
        // Look for declarations like: type *p = malloc(...);
        if node.kind() == "declaration" {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if declarator.kind() == "init_declarator" {
                    if let Some(value) = declarator.child_by_field_name("value") {
                        if Self::is_alloc_call(value, source) {
                            if let Some(decl) = declarator.child_by_field_name("declarator") {
                                if let Some(var_name) =
                                    find_identifier_in_declarator(decl, source)
                                {
                                    results.push((node, var_name));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Also check assignment expressions: p = malloc(...);
        if node.kind() == "expression_statement" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "assignment_expression" {
                    if let Some(right) = child.child_by_field_name("right") {
                        if Self::is_alloc_call(right, source) {
                            if let Some(left) = child.child_by_field_name("left") {
                                let var_name = node_text(left, source).to_string();
                                results.push((node, var_name));
                            }
                        }
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            // Don't recurse into nested functions (shouldn't happen in C, but safe)
            if child.kind() != "function_definition" {
                Self::walk_for_allocs(child, source, results);
            }
        }
    }

    fn is_alloc_call(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "call_expression" {
            if let Some(func) = node.child_by_field_name("function") {
                let fn_name = node_text(func, source);
                return ALLOC_FUNCTIONS.contains(&fn_name);
            }
        }
        // Handle cast expressions: (type *)malloc(...)
        if node.kind() == "cast_expression" {
            if let Some(value) = node.child_by_field_name("value") {
                return Self::is_alloc_call(value, source);
            }
        }
        false
    }

    fn has_null_check_after(
        alloc_node: tree_sitter::Node,
        var_name: &str,
        source: &[u8],
    ) -> bool {
        // Check the next few siblings for an if-statement referencing the variable
        let mut sibling = alloc_node.next_named_sibling();
        let mut checked = 0;
        while let Some(sib) = sibling {
            if checked >= 3 {
                break;
            }
            if sib.kind() == "if_statement" {
                let cond_text = sib
                    .child_by_field_name("condition")
                    .map(|n| node_text(n, source))
                    .unwrap_or("");
                if cond_text.contains(var_name)
                    && (cond_text.contains("NULL")
                        || cond_text.contains("null")
                        || cond_text.contains('!'))
                {
                    return true;
                }
            }
            sibling = sib.next_named_sibling();
            checked += 1;
        }
        false
    }
}

impl Pipeline for UncheckedMallocPipeline {
    fn name(&self) -> &str {
        "unchecked_malloc"
    }

    fn description(&self) -> &str {
        "Detects malloc/calloc/realloc calls without null-check on the returned pointer"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let fn_body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_body_idx);

            if let Some(body_cap) = body_cap {
                let allocs = Self::find_alloc_calls_in_body(body_cap.node, source);

                for (alloc_node, var_name) in allocs {
                    if !Self::has_null_check_after(alloc_node, &var_name, source) {
                        let start = alloc_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unchecked_allocation".to_string(),
                            message: format!(
                                "`{var_name}` allocated without null check — dereference may crash"
                            ),
                            snippet: extract_snippet(source, alloc_node, 1),
                        });
                    }
                }
            }
        }

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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UncheckedMallocPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_unchecked_malloc() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10);
    p[0] = 1;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unchecked_allocation");
        assert!(findings[0].message.contains("p"));
    }

    #[test]
    fn skips_checked_malloc_not_null() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10);
    if (!p) return;
    p[0] = 1;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_checked_malloc_null() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10);
    if (p == NULL) return;
    p[0] = 1;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
