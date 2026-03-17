use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_binary_expression_security_query, compile_direct_call_query,
    compile_method_call_security_query, extract_snippet, find_capture_index, node_text,
};

const SECURITY_NAMES: &[&str] = &[
    "token", "hash", "secret", "password", "hmac", "signature", "digest", "apikey", "api_key",
    "auth", "credential", "nonce",
];

pub struct TimingWeakCryptoPipeline {
    binary_query: Arc<Query>,
    method_call_query: Arc<Query>,
    #[allow(dead_code)]
    direct_call_query: Arc<Query>,
}

impl TimingWeakCryptoPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            binary_query: compile_binary_expression_security_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
            direct_call_query: compile_direct_call_query(language)?,
        })
    }
}

fn is_security_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECURITY_NAMES.iter().any(|s| lower.contains(s))
}

/// Extract the operator from a binary_expression node by checking child(1)
fn get_operator<'a>(node: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
    // In tree-sitter JS, binary_expression children: [left, operator, right]
    // The operator is child(1) (unnamed)
    if node.child_count() >= 3 {
        if let Some(op) = node.child(1) {
            return node_text(op, source);
        }
    }
    ""
}

impl Pipeline for TimingWeakCryptoPipeline {
    fn name(&self) -> &str {
        "timing_weak_crypto"
    }

    fn description(&self) -> &str {
        "Detects timing attacks (=== on secrets), Math.random() for tokens, weak hash algorithms, insecure cipher modes"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Timing attack: === or !== on security-related names
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.binary_query, tree.root_node(), source);
            let left_idx = find_capture_index(&self.binary_query, "left");
            let right_idx = find_capture_index(&self.binary_query, "right");
            let binary_idx = find_capture_index(&self.binary_query, "binary");

            while let Some(m) = matches.next() {
                let left_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == left_idx)
                    .map(|c| c.node);
                let right_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == right_idx)
                    .map(|c| c.node);
                let binary_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == binary_idx)
                    .map(|c| c.node);

                if let (Some(left), Some(right), Some(binary)) =
                    (left_node, right_node, binary_node)
                {
                    let op = get_operator(binary, source);
                    if op == "===" || op == "!==" || op == "==" || op == "!=" {
                        let left_text = node_text(left, source);
                        let right_text = node_text(right, source);

                        if is_security_name(left_text) || is_security_name(right_text) {
                            let start = binary.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "timing_attack_comparison".to_string(),
                                message:
                                    "String comparison on secret/token — use constant-time comparison"
                                        .to_string(),
                                snippet: extract_snippet(source, binary, 1),
                            });
                        }
                    }
                }
            }
        }

        // Math.random() for security, weak hash, insecure cipher
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.method_call_query, tree.root_node(), source);
            let obj_idx = find_capture_index(&self.method_call_query, "obj");
            let method_idx = find_capture_index(&self.method_call_query, "method");
            let args_idx = find_capture_index(&self.method_call_query, "args");
            let call_idx = find_capture_index(&self.method_call_query, "call");

            while let Some(m) = matches.next() {
                let obj_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == obj_idx)
                    .map(|c| c.node);
                let method_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_idx)
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

                if let (Some(obj), Some(method), Some(args), Some(call)) =
                    (obj_node, method_node, args_node, call_node)
                {
                    let obj_name = node_text(obj, source);
                    let method_name = node_text(method, source);

                    // Math.random() in security context
                    if obj_name == "Math" && method_name == "random" {
                        // Check if the result is assigned to a security-related variable
                        if let Some(parent) = call.parent() {
                            if let Some(gp) = parent.parent() {
                                if gp.kind() == "variable_declarator"
                                    || gp.kind() == "assignment_expression"
                                {
                                    let gp_text = node_text(gp, source);
                                    if is_security_name(gp_text) {
                                        let start = call.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "warning".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "weak_random_token".to_string(),
                                            message: "`Math.random()` for security value — use crypto.randomBytes() instead".to_string(),
                                            snippet: extract_snippet(source, gp, 1),
                                        });
                                    }
                                }
                            }
                        }
                    }

                    // createHash('md5') / createHash('sha1')
                    if method_name == "createHash" {
                        if let Some(first_arg) = args.named_child(0) {
                            let algo = node_text(first_arg, source);
                            if algo.contains("md5") || algo.contains("sha1") {
                                let start = call.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "weak_hash_algorithm".to_string(),
                                    message: format!(
                                        "Weak hash algorithm {} — use SHA-256 or stronger",
                                        algo.trim_matches(|c| c == '\'' || c == '"')
                                    ),
                                    snippet: extract_snippet(source, call, 1),
                                });
                            }
                        }
                    }

                    // createCipheriv with ECB mode or literal/zero IV
                    if method_name == "createCipheriv" {
                        if let Some(first_arg) = args.named_child(0) {
                            let algo = node_text(first_arg, source);
                            if algo.contains("ecb") {
                                let start = call.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "insecure_cipher_mode".to_string(),
                                    message:
                                        "ECB cipher mode — use CBC or GCM instead".to_string(),
                                    snippet: extract_snippet(source, call, 1),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let lang = Language::JavaScript;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&lang.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TimingWeakCryptoPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_timing_attack_on_token() {
        let src = "if (token === userToken) { grant(); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "timing_attack_comparison");
    }

    #[test]
    fn detects_timing_attack_on_hash() {
        let src = "if (computedHash !== expectedHash) { reject(); }";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "timing_attack_comparison"));
    }

    #[test]
    fn ignores_comparison_on_normal_vars() {
        let src = "if (count === 5) { }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_weak_hash_md5() {
        let src = r#"crypto.createHash("md5");"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_hash_algorithm");
    }

    #[test]
    fn detects_weak_hash_sha1() {
        let src = r#"crypto.createHash("sha1");"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_hash_algorithm");
    }

    #[test]
    fn detects_ecb_cipher_mode() {
        let src = r#"crypto.createCipheriv("aes-128-ecb", key, iv);"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "insecure_cipher_mode");
    }

    #[test]
    fn ignores_safe_cipher() {
        let src = r#"crypto.createCipheriv("aes-256-gcm", key, iv);"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
