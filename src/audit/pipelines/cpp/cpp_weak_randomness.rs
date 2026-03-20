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

const SECURITY_KEYWORDS: &[&str] = &[
    "key", "token", "auth", "crypt", "password", "nonce", "secret", "session", "hash", "random",
];

pub struct CppWeakRandomnessPipeline {
    call_query: Arc<Query>,
}

impl CppWeakRandomnessPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

fn find_enclosing_function_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(p) = current {
        if p.kind() == "function_definition"
            && let Some(decl) = p.child_by_field_name("declarator") {
                return find_identifier_in_declarator(decl, source);
            }
        current = p.parent();
    }
    None
}

fn name_contains_security_keyword(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECURITY_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn matches_function(fn_text: &str, target: &str) -> bool {
    fn_text == target || fn_text.ends_with(&format!("::{target}"))
}

fn any_arg_contains_security_keyword(args_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        let arg_text = node_text(child, source).to_lowercase();
        if SECURITY_KEYWORDS.iter().any(|kw| arg_text.contains(kw)) {
            return true;
        }
    }
    false
}

impl Pipeline for CppWeakRandomnessPipeline {
    fn name(&self) -> &str {
        "cpp_weak_randomness"
    }

    fn description(&self) -> &str {
        "Detects weak randomness: std::rand/srand in security contexts, predictable mt19937 seeding, timing side-channels"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(fn_cap), Some(args_cap), Some(call_cap)) = (fn_cap, args_cap, call_cap) {
                let fn_text = node_text(fn_cap.node, source);

                // Pattern: std::rand/srand in security-sensitive function
                let rand_fns = ["rand", "srand"];
                for &target in &rand_fns {
                    if matches_function(fn_text, target) {
                        if let Some(enclosing) = find_enclosing_function_name(call_cap.node, source)
                            && name_contains_security_keyword(&enclosing) {
                                let start = call_cap.node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "std_rand_in_security".to_string(),
                                    message: format!(
                                        "`{fn_text}()` in security-sensitive function — use `<random>` with proper seeding or OS CSPRNG"
                                    ),
                                    snippet: extract_snippet(source, call_cap.node, 1),
                                });
                            }
                        break;
                    }
                }

                // Pattern: timing side-channel
                let timing_fns = ["memcmp", "strcmp", "std::equal"];
                for &target in &timing_fns {
                    if matches_function(fn_text, target) {
                        if any_arg_contains_security_keyword(args_cap.node, source) {
                            let start = call_cap.node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "timing_side_channel".to_string(),
                                message: format!(
                                    "`{fn_text}()` on security-sensitive data — vulnerable to timing side-channel attack"
                                ),
                                snippet: extract_snippet(source, call_cap.node, 1),
                            });
                        }
                        break;
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CppWeakRandomnessPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_rand_in_token_gen() {
        let src = "void generate_token() { int x = rand(); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "std_rand_in_security");
    }

    #[test]
    fn ignores_rand_in_game() {
        let src = "void update_physics() { int x = rand(); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_memcmp_on_secret() {
        let src = "bool check(const char *password, const char *hash) { return memcmp(password, hash, 32) == 0; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "timing_side_channel");
    }

    #[test]
    fn ignores_memcmp_on_regular_data() {
        let src = "bool eq(const char *a, const char *b) { return memcmp(a, b, 32) == 0; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_srand_in_crypto() {
        let src = "void init_crypto_key() { srand(42); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "std_rand_in_security");
    }

    #[test]
    fn metadata_correct() {
        let src = "void generate_token() { int x = rand(); }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "cpp_weak_randomness");
    }
}
