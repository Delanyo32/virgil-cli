use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index,
    find_identifier_in_declarator, node_text,
};

const WEAK_RAND_FUNCTIONS: &[&str] = &["rand", "random"];
const TIMING_UNSAFE_FUNCTIONS: &[&str] = &["memcmp", "strcmp"];
const SECURITY_KEYWORDS: &[&str] = &[
    "key", "token", "auth", "crypt", "password", "nonce", "secret", "session", "hash", "random",
];
const SECRET_KEYWORDS: &[&str] = &[
    "key", "token", "password", "secret", "hash", "digest", "hmac",
];

pub struct CWeakRandomnessPipeline {
    call_query: Arc<Query>,
}

impl CWeakRandomnessPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

fn find_enclosing_function_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(p) = current {
        if p.kind() == "function_definition" {
            if let Some(decl) = p.child_by_field_name("declarator") {
                return find_identifier_in_declarator(decl, source);
            }
        }
        current = p.parent();
    }
    None
}

fn contains_security_keyword(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECURITY_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn contains_secret_keyword(text: &str) -> bool {
    let lower = text.to_lowercase();
    SECRET_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

impl Pipeline for CWeakRandomnessPipeline {
    fn name(&self) -> &str {
        "c_weak_randomness"
    }

    fn description(&self) -> &str {
        "Detects weak randomness: rand()/srand(time(NULL)) in security contexts, memcmp/strcmp on secrets"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");
        let args_idx = find_capture_index(&self.call_query, "args");

        while let Some(m) = matches.next() {
            let fn_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);
            let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);

            if let (Some(fn_cap), Some(call_cap), Some(args_cap)) = (fn_cap, call_cap, args_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");
                let call_node = call_cap.node;
                let args_node = args_cap.node;

                // Pattern: rand()/random() in security-sensitive function
                if WEAK_RAND_FUNCTIONS.contains(&fn_name) {
                    if let Some(enclosing_fn) = find_enclosing_function_name(call_node, source) {
                        if contains_security_keyword(&enclosing_fn) {
                            let start = call_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "rand_in_security_context".to_string(),
                                message: format!(
                                    "`rand()` used in security-sensitive function `{enclosing_fn}` — use a CSPRNG instead"
                                ),
                                snippet: extract_snippet(source, call_node, 1),
                            });
                        }
                    }
                }

                // Pattern: srand(time(NULL))
                if fn_name == "srand" {
                    let mut walker = args_node.walk();
                    let named_args: Vec<tree_sitter::Node> =
                        args_node.named_children(&mut walker).collect();
                    if let Some(first_arg) = named_args.first() {
                        let arg_text = node_text(*first_arg, source);
                        if arg_text.contains("time") {
                            let start = call_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "srand_time_null".to_string(),
                                message: "`srand(time(NULL))` provides predictable seed — use a CSPRNG for security-sensitive randomness".to_string(),
                                snippet: extract_snippet(source, call_node, 1),
                            });
                        }
                    }
                }

                // Pattern: memcmp/strcmp on secrets
                if TIMING_UNSAFE_FUNCTIONS.contains(&fn_name) {
                    let mut walker = args_node.walk();
                    let named_args: Vec<tree_sitter::Node> =
                        args_node.named_children(&mut walker).collect();
                    let any_secret = named_args.iter().any(|arg| {
                        let arg_text = node_text(*arg, source);
                        contains_secret_keyword(arg_text)
                    });
                    if any_secret {
                        let start = call_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "memcmp_secrets".to_string(),
                            message: format!(
                                "`{fn_name}()` on security-sensitive data is vulnerable to timing side-channel — use constant-time comparison"
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CWeakRandomnessPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_rand_in_auth() {
        let src = "void generate_auth_token() { int x = rand(); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "rand_in_security_context");
        assert!(findings[0].message.contains("generate_auth_token"));
    }

    #[test]
    fn ignores_rand_in_game() {
        let src = "void update_game() { int x = rand(); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_srand_time() {
        let src = "void f() { srand(time(NULL)); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "srand_time_null");
    }

    #[test]
    fn detects_memcmp_password() {
        let src = "void f(char *password) { memcmp(password, stored_hash, 32); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "memcmp_secrets");
        assert!(findings[0].message.contains("memcmp"));
    }
}
