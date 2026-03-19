use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_field_access_query, compile_method_invocation_with_object_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct WeakCryptographyPipeline {
    method_query: Arc<Query>,
    field_query: Arc<Query>,
}

impl WeakCryptographyPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_invocation_with_object_query()?,
            field_query: compile_field_access_query()?,
        })
    }
}

impl Pipeline for WeakCryptographyPipeline {
    fn name(&self) -> &str {
        "weak_cryptography"
    }

    fn description(&self) -> &str {
        "Detects weak cryptography: MD5, SHA-1, ECB mode, Math.random for security, timing-unsafe comparison"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_method_calls(tree, source, file_path, &mut findings);
        self.check_field_access(tree, source, file_path, &mut findings);
        findings
    }
}

impl WeakCryptographyPipeline {
    fn check_method_calls(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let obj_idx = find_capture_index(&self.method_query, "object");
        let method_idx = find_capture_index(&self.method_query, "method_name");
        let args_idx = find_capture_index(&self.method_query, "args");
        let inv_idx = find_capture_index(&self.method_query, "invocation");

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
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(obj_node), Some(method_node), Some(args_node), Some(inv_node)) =
                (obj_node, method_node, args_node, inv_node)
            {
                let obj_name = node_text(obj_node, source);
                let method_name = node_text(method_node, source);
                let args_text = node_text(args_node, source);

                // MessageDigest.getInstance("MD5") or ("SHA-1")
                if obj_name == "MessageDigest" && method_name == "getInstance" {
                    if args_text.contains("\"MD5\"")
                        || args_text.contains("\"SHA-1\"")
                        || args_text.contains("\"SHA1\"")
                    {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "weak_hash_algorithm".to_string(),
                            message: format!(
                                "MessageDigest.getInstance() uses a weak hash algorithm — use SHA-256 or stronger"
                            ),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }

                // Cipher.getInstance("AES/ECB/...")
                if obj_name == "Cipher" && method_name == "getInstance" {
                    if args_text.contains("ECB") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "ecb_mode".to_string(),
                            message: "Cipher.getInstance() uses ECB mode — use CBC or GCM instead"
                                .to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }

                // String.equals on hash/token comparison (timing attack)
                if method_name == "equals" {
                    let inv_text = node_text(inv_node, source);
                    let looks_like_secret =
                        ["hash", "token", "secret", "password", "digest", "signature"]
                            .iter()
                            .any(|kw| inv_text.to_lowercase().contains(kw));
                    if looks_like_secret {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "timing_unsafe_comparison".to_string(),
                            message: "String.equals() on secret value is timing-unsafe — use MessageDigest.isEqual()".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }
            }
        }
    }

    fn check_field_access(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let obj_idx = find_capture_index(&self.field_query, "object");
        let field_idx = find_capture_index(&self.field_query, "field_name");
        let access_idx = find_capture_index(&self.field_query, "access");

        while let Some(m) = matches.next() {
            let obj_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == obj_idx)
                .map(|c| c.node);
            let field_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_idx)
                .map(|c| c.node);
            let access_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == access_idx)
                .map(|c| c.node);

            if let (Some(obj_node), Some(field_node), Some(access_node)) =
                (obj_node, field_node, access_node)
            {
                let obj_name = node_text(obj_node, source);
                let field_name = node_text(field_node, source);

                // Math.random() used as a field access (it's actually a method but detect via parent)
                if obj_name == "Math" && field_name == "random" {
                    // Check if this is used in a security-sensitive context
                    if let Some(parent) = access_node.parent() {
                        let parent_text = node_text(parent, source);
                        let in_security_context =
                            ["token", "secret", "key", "nonce", "iv", "salt", "password"]
                                .iter()
                                .any(|kw| {
                                    // Check surrounding code
                                    let line_start = access_node.start_position().row;
                                    let source_str = std::str::from_utf8(source).unwrap_or("");
                                    if let Some(line) = source_str.lines().nth(line_start) {
                                        line.to_lowercase().contains(kw)
                                    } else {
                                        parent_text.to_lowercase().contains(kw)
                                    }
                                });

                        if in_security_context {
                            let start = access_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "insecure_random".to_string(),
                                message: "Math.random() is not cryptographically secure — use SecureRandom".to_string(),
                                snippet: extract_snippet(source, access_node, 1),
                            });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = WeakCryptographyPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_md5() {
        let src = r#"class Foo {
    void hash() {
        MessageDigest.getInstance("MD5");
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_hash_algorithm");
    }

    #[test]
    fn detects_sha1() {
        let src = r#"class Foo {
    void hash() {
        MessageDigest.getInstance("SHA-1");
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_hash_algorithm");
    }

    #[test]
    fn detects_ecb_mode() {
        let src = r#"class Foo {
    void encrypt() {
        Cipher.getInstance("AES/ECB/PKCS5Padding");
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ecb_mode");
    }

    #[test]
    fn detects_timing_unsafe() {
        let src = r#"class Foo {
    boolean verify(String hash, String input) {
        return hash.equals(input);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "timing_unsafe_comparison");
    }

    #[test]
    fn ignores_sha256() {
        let src = r#"class Foo {
    void hash() {
        MessageDigest.getInstance("SHA-256");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_gcm_mode() {
        let src = r#"class Foo {
    void encrypt() {
        Cipher.getInstance("AES/GCM/NoPadding");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
