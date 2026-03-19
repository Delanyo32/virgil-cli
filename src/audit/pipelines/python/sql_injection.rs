use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const SQL_METHODS: &[&str] = &["execute", "executemany", "executescript"];

pub struct SqlInjectionPipeline {
    call_query: Arc<Query>,
}

impl SqlInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for SqlInjectionPipeline {
    fn name(&self) -> &str {
        "sql_injection"
    }

    fn description(&self) -> &str {
        "Detects SQL injection risks: f-strings, format(), %, or concatenation in execute() calls"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(call_node)) =
                (fn_node, args_node, call_node)
            {
                if fn_node.kind() != "attribute" {
                    continue;
                }

                let attr = fn_node
                    .child_by_field_name("attribute")
                    .map(|n| node_text(n, source));
                let method_name = match attr {
                    Some(name) if SQL_METHODS.contains(&name) => name,
                    _ => continue,
                };

                // Check the first argument for unsafe patterns
                if let Some(first_arg) = args_node.named_child(0) {
                    let kind = first_arg.kind();

                    // Check for f-string: string node with interpolation children
                    let is_fstring = kind == "string" && has_interpolation(first_arg);

                    let (pattern, msg) = if is_fstring {
                        (
                            "sql_fstring",
                            format!(
                                "`.{method_name}()` with f-string — use parameterized queries instead"
                            ),
                        )
                    } else {
                        match kind {
                            // %-format: binary_operator with % on a string
                            "binary_operator" => {
                                let text = node_text(first_arg, source);
                                if text.contains('%') {
                                    (
                                        "sql_percent_format",
                                        format!(
                                            "`.{method_name}()` with %-formatting — use parameterized queries instead"
                                        ),
                                    )
                                } else if text.contains('+') {
                                    (
                                        "sql_concat",
                                        format!(
                                            "`.{method_name}()` with string concatenation — use parameterized queries instead"
                                        ),
                                    )
                                } else {
                                    continue;
                                }
                            }
                            // .format() call: the arg itself is a call with attribute .format
                            "call" => {
                                let arg_text = node_text(first_arg, source);
                                if arg_text.contains(".format(") {
                                    (
                                        "sql_format",
                                        format!(
                                            "`.{method_name}()` with .format() — use parameterized queries instead"
                                        ),
                                    )
                                } else {
                                    continue;
                                }
                            }
                            _ => continue,
                        }
                    };

                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: msg,
                        snippet: extract_snippet(source, call_node, 1),
                    });
                }
            }
        }

        findings
    }
}

/// Checks if a Python `string` node contains `interpolation` children (i.e., is an f-string).
fn has_interpolation(node: tree_sitter::Node) -> bool {
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            if child.kind() == "interpolation" {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SqlInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_fstring_in_execute() {
        let src = "cursor.execute(f\"SELECT * FROM users WHERE id = {id}\")";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_fstring");
    }

    #[test]
    fn detects_percent_format() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = %s\" % user_id)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_percent_format");
    }

    #[test]
    fn detects_format_call() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = {}\".format(user_id))";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_format");
    }

    #[test]
    fn detects_concatenation() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = \" + user_id)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sql_concat");
    }

    #[test]
    fn ignores_parameterized_query() {
        let src = "cursor.execute(\"SELECT * FROM users WHERE id = ?\", (user_id,))";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
