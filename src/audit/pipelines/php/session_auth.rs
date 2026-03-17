use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_function_call_query, extract_snippet, find_capture_index, node_text};

const WEAK_HASH_FUNCTIONS: &[&str] = &["md5", "sha1"];
const WEAK_RANDOM_FUNCTIONS: &[&str] = &["uniqid", "rand", "mt_rand", "microtime"];

pub struct SessionAuthPipeline {
    call_query: Arc<Query>,
}

impl SessionAuthPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_function_call_query()?,
        })
    }
}

impl Pipeline for SessionAuthPipeline {
    fn name(&self) -> &str {
        "session_auth"
    }

    fn description(&self) -> &str {
        "Detects weak session/auth patterns: md5/sha1 for hashing, weak random for tokens"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == fn_name_idx).map(|c| c.node);
            let args_node = m.captures.iter().find(|c| c.index as usize == args_idx).map(|c| c.node);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx).map(|c| c.node);

            if let (Some(name_node), Some(args_node), Some(call_node)) = (name_node, args_node, call_node) {
                let fn_name = node_text(name_node, source);

                if !WEAK_HASH_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                // Get the actual expression from the first argument (drill into `argument` wrapper)
                let first_expr = args_node.named_child(0).and_then(|arg| {
                    if arg.kind() == "argument" {
                        arg.named_child(0)
                    } else {
                        Some(arg)
                    }
                });

                if let Some(first_arg) = first_expr {
                    // Check if the argument is a call to a weak random function
                    // Pattern: md5(uniqid()), sha1(rand()), etc.
                    if first_arg.kind() == "function_call_expression" {
                        if let Some(inner_fn) = first_arg.child_by_field_name("function") {
                            let inner_name = node_text(inner_fn, source);
                            if WEAK_RANDOM_FUNCTIONS.contains(&inner_name) {
                                let start = call_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "weak_random_token".to_string(),
                                    message: format!(
                                        "`{fn_name}({inner_name}())` generates weak tokens — use random_bytes() or openssl_random_pseudo_bytes()"
                                    ),
                                    snippet: extract_snippet(source, call_node, 1),
                                });
                                continue;
                            }
                        }
                    }

                    // Check for password hashing pattern: md5($password) / sha1($password)
                    // Heuristic: argument variable name contains "pass" or "pw"
                    let arg_text = node_text(first_arg, source);
                    let arg_lower = arg_text.to_lowercase();
                    if arg_lower.contains("pass") || arg_lower.contains("pw") {
                        let start = call_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "weak_password_hash".to_string(),
                            message: format!(
                                "`{fn_name}()` for password hashing — use password_hash() with PASSWORD_BCRYPT instead"
                            ),
                            snippet: extract_snippet(source, call_node, 1),
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SessionAuthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_md5_uniqid() {
        let src = "<?php\n$token = md5(uniqid());\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_random_token");
    }

    #[test]
    fn detects_sha1_rand() {
        let src = "<?php\n$token = sha1(rand());\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_random_token");
    }

    #[test]
    fn detects_md5_password() {
        let src = "<?php\n$hash = md5($password);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_password_hash");
    }

    #[test]
    fn ignores_password_hash() {
        let src = "<?php\npassword_hash($pw, PASSWORD_BCRYPT);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_md5_non_password() {
        let src = "<?php\n$hash = md5($file_content);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
