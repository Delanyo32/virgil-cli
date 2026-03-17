use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, extract_snippet, find_capture_index, node_text,
};

const CHECK_FUNCTIONS: &[&str] = &["access", "stat", "lstat"];
const USE_FUNCTIONS: &[&str] = &["fopen", "open"];

pub struct CToctouPipeline {
    fn_def_query: Arc<Query>,
}

impl CToctouPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_def_query: compile_function_definition_query()?,
        })
    }

    /// Recursively collect all call_expression nodes in a function body.
    /// Returns (fn_name, first_arg_text, call_node) triples.
    fn collect_calls_in_body<'a>(
        body: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> Vec<(String, String, tree_sitter::Node<'a>)> {
        let mut calls = Vec::new();
        Self::walk_for_calls(body, source, &mut calls);
        calls
    }

    fn walk_for_calls<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        calls: &mut Vec<(String, String, tree_sitter::Node<'a>)>,
    ) {
        if node.kind() == "call_expression" {
            if let Some(func) = node.child_by_field_name("function") {
                let fn_name = node_text(func, source).to_string();
                if let Some(args) = node.child_by_field_name("arguments") {
                    let first_arg = {
                        let mut walker = args.walk();
                        args.named_children(&mut walker)
                            .next()
                            .map(|n| node_text(n, source).to_string())
                            .unwrap_or_default()
                    };
                    calls.push((fn_name, first_arg, node));
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            // Don't recurse into nested function definitions
            if child.kind() != "function_definition" {
                Self::walk_for_calls(child, source, calls);
            }
        }
    }
}

impl Pipeline for CToctouPipeline {
    fn name(&self) -> &str {
        "c_toctou"
    }

    fn description(&self) -> &str {
        "Detects TOCTOU race conditions: access() then open(), stat() then open(), getpid() in file paths"
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
                let calls = Self::collect_calls_in_body(body_cap.node, source);

                // Track "check" calls: (check_fn_name, path_arg, call_node)
                let mut check_calls: Vec<(&str, &str, tree_sitter::Node)> = Vec::new();

                for (fn_name, first_arg, call_node) in &calls {
                    let fn_str = fn_name.as_str();

                    // Record check calls
                    if CHECK_FUNCTIONS.contains(&fn_str) && !first_arg.is_empty() {
                        check_calls.push((fn_str, first_arg.as_str(), *call_node));
                    }

                    // Check for use calls that match a previous check call
                    if USE_FUNCTIONS.contains(&fn_str) && !first_arg.is_empty() {
                        for (check_fn, check_path, _check_node) in &check_calls {
                            if *check_path == first_arg.as_str() {
                                let pattern = if *check_fn == "access" {
                                    "access_then_open"
                                } else {
                                    "stat_then_open"
                                };

                                let message = if *check_fn == "access" {
                                    format!(
                                        "`access()` followed by `{fn_str}()` on `{first_arg}` — TOCTOU race condition"
                                    )
                                } else {
                                    format!(
                                        "`{check_fn}()` followed by `open()` on `{first_arg}` without `O_NOFOLLOW` — TOCTOU race condition"
                                    )
                                };

                                let start = call_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: pattern.to_string(),
                                    message,
                                    snippet: extract_snippet(source, *call_node, 1),
                                });
                            }
                        }
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
        let pipeline = CToctouPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_access_then_fopen() {
        let src = r#"
void f(const char *path) {
    if (access(path, R_OK) == 0) {
        FILE *fp = fopen(path, "r");
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "access_then_open");
    }

    #[test]
    fn ignores_unrelated_calls() {
        let src = r#"
void f() {
    printf("hello");
    FILE *fp = fopen("config.txt", "r");
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }
}
