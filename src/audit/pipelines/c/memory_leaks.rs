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

const ALLOC_FUNCTIONS: &[&str] = &["malloc", "calloc"];

pub struct MemoryLeaksPipeline {
    fn_def_query: Arc<Query>,
}

impl MemoryLeaksPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_def_query: compile_function_definition_query()?,
        })
    }

    fn scan_body<'a>(
        body: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> (Vec<(tree_sitter::Node<'a>, String)>, bool, Vec<String>) {
        // Returns: (alloc_nodes_with_var, has_free, returned_vars)
        let mut allocs = Vec::new();
        let mut has_free = false;
        let mut returned_vars = Vec::new();
        Self::walk_body(body, source, &mut allocs, &mut has_free, &mut returned_vars);
        (allocs, has_free, returned_vars)
    }

    fn walk_body<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        allocs: &mut Vec<(tree_sitter::Node<'a>, String)>,
        has_free: &mut bool,
        returned_vars: &mut Vec<String>,
    ) {
        // Check for allocation in declarations
        if node.kind() == "declaration"
            && let Some(declarator) = node.child_by_field_name("declarator")
                && declarator.kind() == "init_declarator"
                    && let Some(value) = declarator.child_by_field_name("value")
                        && Self::is_alloc_call(value, source)
                            && let Some(decl) = declarator.child_by_field_name("declarator")
                                && let Some(var_name) = find_identifier_in_declarator(decl, source)
                                {
                                    allocs.push((node, var_name));
                                }

        // Check for free() calls
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
                && node_text(func, source) == "free" {
                    *has_free = true;
                }

        // Check for return statements
        if node.kind() == "return_statement" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    returned_vars.push(node_text(child, source).to_string());
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_body(child, source, allocs, has_free, returned_vars);
        }
    }

    fn is_alloc_call(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function") {
                let fn_name = node_text(func, source);
                return ALLOC_FUNCTIONS.contains(&fn_name);
            }
        if node.kind() == "cast_expression"
            && let Some(value) = node.child_by_field_name("value") {
                return Self::is_alloc_call(value, source);
            }
        false
    }
}

impl Pipeline for MemoryLeaksPipeline {
    fn name(&self) -> &str {
        "memory_leaks"
    }

    fn description(&self) -> &str {
        "Detects malloc/calloc allocations without corresponding free in the same function"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let fn_body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

            if let Some(body_cap) = body_cap {
                let (allocs, has_free, returned_vars) = Self::scan_body(body_cap.node, source);

                // If there's a free() anywhere in the function, skip
                if has_free {
                    continue;
                }

                // If no allocations, skip
                if allocs.is_empty() {
                    continue;
                }

                for (alloc_node, var_name) in allocs {
                    // Skip if the allocated pointer is returned
                    if returned_vars.contains(&var_name) {
                        continue;
                    }

                    let start = alloc_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "potential_memory_leak".to_string(),
                        message: format!(
                            "`{var_name}` is allocated but never freed in this function"
                        ),
                        snippet: extract_snippet(source, alloc_node, 1),
                    });
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
        let pipeline = MemoryLeaksPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_missing_free() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10);
    p[0] = 1;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "potential_memory_leak");
        assert!(findings[0].message.contains("p"));
    }

    #[test]
    fn skips_with_free() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10);
    p[0] = 1;
    free(p);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_returned_pointer() {
        let src = r#"
int *create() {
    int *p = malloc(sizeof(int));
    return p;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
