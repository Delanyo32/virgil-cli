use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, extract_snippet, find_capture_index, node_text,
};

pub struct CMemoryMismanagementPipeline {
    fn_def_query: Arc<Query>,
}

impl CMemoryMismanagementPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_def_query: compile_function_definition_query()?,
        })
    }

    /// Extract a call from an expression_statement: returns (fn_name, first_arg_text).
    fn extract_call(node: tree_sitter::Node, source: &[u8]) -> Option<(String, String)> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call_expression"
                && let Some(func) = child.child_by_field_name("function") {
                    let fn_name = node_text(func, source).to_string();
                    if let Some(args) = child.child_by_field_name("arguments") {
                        let first_arg = args
                            .named_child(0)
                            .map(|n| node_text(n, source).to_string())
                            .unwrap_or_default();
                        return Some((fn_name, first_arg));
                    }
                }
        }
        None
    }

    /// Extract an assignment where the right side is a realloc() call.
    /// Returns (lhs_name, first_arg_of_realloc).
    fn extract_realloc_assign(node: tree_sitter::Node, source: &[u8]) -> Option<(String, String)> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "assignment_expression" {
                let lhs = child.child_by_field_name("left")?;
                let rhs = child.child_by_field_name("right")?;

                if rhs.kind() == "call_expression"
                    && let Some(func) = rhs.child_by_field_name("function")
                        && node_text(func, source) == "realloc" {
                            let lhs_name = node_text(lhs, source).to_string();
                            if let Some(args) = rhs.child_by_field_name("arguments") {
                                let first_arg = args
                                    .named_child(0)
                                    .map(|n| node_text(n, source).to_string())
                                    .unwrap_or_default();
                                return Some((lhs_name, first_arg));
                            }
                        }
            }
        }
        None
    }

    /// Walk a function body looking for use-after-free, double free, and realloc overwrite.
    fn scan_body_for_issues(
        body: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut freed_vars: Vec<String> = Vec::new();

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            let text = node_text(child, source);

            // Check for free() calls and realloc assignments in expression_statement nodes
            if child.kind() == "expression_statement" {
                if let Some((fn_name, arg)) = Self::extract_call(child, source)
                    && fn_name == "free" {
                        if freed_vars.contains(&arg) {
                            // double free
                            let start = child.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "double_free".to_string(),
                                message: format!("double `free()` on `{arg}` — undefined behavior"),
                                snippet: extract_snippet(source, child, 1),
                            });
                        } else {
                            freed_vars.push(arg);
                        }
                        continue;
                    }

                // Check for assignment p = realloc(p, n)
                if let Some((lhs, first_arg)) = Self::extract_realloc_assign(child, source)
                    && lhs == first_arg {
                        let start = child.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "realloc_overwrite".to_string(),
                            message: format!(
                                "`{lhs} = realloc({lhs}, ...)` loses original pointer on failure — use a temporary"
                            ),
                            snippet: extract_snippet(source, child, 1),
                        });
                    }
            }

            // Check for use of freed variable in any subsequent statement
            if !freed_vars.is_empty() {
                // Skip declaration nodes (they shadow, not use)
                if child.kind() == "expression_statement" {
                    // Only flag if it's NOT another free call on the same var
                    let is_free_call = Self::extract_call(child, source)
                        .map(|(name, _)| name == "free")
                        .unwrap_or(false);
                    if !is_free_call {
                        for var in &freed_vars {
                            if text.contains(var.as_str()) {
                                let start = child.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: pipeline_name.to_string(),
                                    pattern: "use_after_free".to_string(),
                                    message: format!(
                                        "`{var}` used after `free()` — undefined behavior"
                                    ),
                                    snippet: extract_snippet(source, child, 1),
                                });
                                break;
                            }
                        }
                    }
                } else if child.kind() == "return_statement" || child.kind() == "if_statement" {
                    for var in &freed_vars {
                        if text.contains(var.as_str()) {
                            let start = child.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "use_after_free".to_string(),
                                message: format!(
                                    "`{var}` used after `free()` — undefined behavior"
                                ),
                                snippet: extract_snippet(source, child, 1),
                            });
                            break;
                        }
                    }
                }
            }
        }

        findings
    }
}

impl Pipeline for CMemoryMismanagementPipeline {
    fn name(&self) -> &str {
        "c_memory_mismanagement"
    }

    fn description(&self) -> &str {
        "Detects memory mismanagement: use-after-free, double free, returning stack arrays, realloc overwrite"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let fn_body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

            if let Some(body_cap) = body_cap {
                let mut body_findings =
                    Self::scan_body_for_issues(body_cap.node, source, file_path, self.name());
                findings.append(&mut body_findings);
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
        let pipeline = CMemoryMismanagementPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_use_after_free() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int));
    free(p);
    *p = 42;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "use_after_free");
        assert!(findings[0].message.contains("p"));
    }

    #[test]
    fn detects_double_free() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int));
    free(p);
    free(p);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "double_free");
        assert!(findings[0].message.contains("p"));
    }

    #[test]
    fn ignores_safe_free() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int));
    *p = 42;
    free(p);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_realloc_overwrite() {
        let src = r#"
void f() {
    char *p = malloc(10);
    p = realloc(p, 20);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "realloc_overwrite");
        assert!(findings[0].message.contains("p"));
    }
}
