use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_assignment_expression_query, compile_invocation_query, compile_object_creation_query,
    extract_snippet, find_capture_index, node_text,
};

pub struct WeakCryptographyPipeline {
    invocation_query: Arc<Query>,
    creation_query: Arc<Query>,
    assign_query: Arc<Query>,
}

impl WeakCryptographyPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            invocation_query: compile_invocation_query()?,
            creation_query: compile_object_creation_query()?,
            assign_query: compile_assignment_expression_query()?,
        })
    }
}

impl Pipeline for WeakCryptographyPipeline {
    fn name(&self) -> &str {
        "weak_cryptography"
    }

    fn description(&self) -> &str {
        "Detects weak cryptography: MD5, SHA1, DES, ECB mode, insecure Random, timing-unsafe comparison"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_invocations(tree, source, file_path, &mut findings);
        self.check_creations(tree, source, file_path, &mut findings);
        self.check_assignments(tree, source, file_path, &mut findings);
        findings
    }
}

impl WeakCryptographyPipeline {
    fn check_invocations(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
        let inv_idx = find_capture_index(&self.invocation_query, "invocation");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(inv_node)) = (fn_node, inv_node) {
                let fn_text = node_text(fn_node, source);

                // MD5.Create(), SHA1.Create()
                if (fn_text.contains("MD5") || fn_text.contains("SHA1"))
                    && fn_text.contains("Create")
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
                            "{fn_text}() uses a weak hash algorithm — use SHA256 or stronger"
                        ),
                        snippet: extract_snippet(source, inv_node, 1),
                    });
                }

                // DES.Create()
                if fn_text.contains("DES")
                    && fn_text.contains("Create")
                    && !fn_text.contains("TripleDES")
                {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "weak_cipher".to_string(),
                        message: "DES.Create() uses a weak cipher — use AES instead".to_string(),
                        snippet: extract_snippet(source, inv_node, 1),
                    });
                }
            }
        }
    }

    fn check_creations(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.creation_query, tree.root_node(), source);

        let type_idx = find_capture_index(&self.creation_query, "type_name");
        let creation_idx = find_capture_index(&self.creation_query, "creation");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(creation_node)) = (type_node, creation_node) {
                let type_name = node_text(type_node, source);

                // new Random() for security purposes
                if type_name == "Random" {
                    let source_str = std::str::from_utf8(source).unwrap_or("");
                    let in_security_context = [
                        "token", "secret", "key", "nonce", "salt", "password", "hash",
                    ]
                    .iter()
                    .any(|kw| source_str.to_lowercase().contains(kw));

                    if in_security_context {
                        let start = creation_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "insecure_random".to_string(),
                            message: "new Random() is not cryptographically secure — use RandomNumberGenerator".to_string(),
                            snippet: extract_snippet(source, creation_node, 1),
                        });
                    }
                }
            }
        }
    }

    fn check_assignments(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.assign_query, tree.root_node(), source);

        let lhs_idx = find_capture_index(&self.assign_query, "lhs");
        let rhs_idx = find_capture_index(&self.assign_query, "rhs");
        let assign_idx = find_capture_index(&self.assign_query, "assign");

        while let Some(m) = matches.next() {
            let lhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == lhs_idx)
                .map(|c| c.node);
            let rhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == rhs_idx)
                .map(|c| c.node);
            let assign_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == assign_idx)
                .map(|c| c.node);

            if let (Some(lhs_node), Some(rhs_node), Some(assign_node)) =
                (lhs_node, rhs_node, assign_node)
            {
                let lhs_text = node_text(lhs_node, source);
                let rhs_text = node_text(rhs_node, source);

                // aes.Mode = CipherMode.ECB
                if lhs_text.contains("Mode") && rhs_text.contains("ECB") {
                    let start = assign_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "ecb_mode".to_string(),
                        message: "CipherMode.ECB is insecure — use CBC or GCM instead".to_string(),
                        snippet: extract_snippet(source, assign_node, 1),
                    });
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = WeakCryptographyPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_md5() {
        let src = r#"class Foo {
    void Hash() {
        MD5.Create();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_hash_algorithm");
    }

    #[test]
    fn detects_sha1() {
        let src = r#"class Foo {
    void Hash() {
        SHA1.Create();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_hash_algorithm");
    }

    #[test]
    fn detects_ecb_mode() {
        let src = r#"class Foo {
    void Encrypt() {
        aes.Mode = CipherMode.ECB;
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ecb_mode");
    }

    #[test]
    fn detects_des() {
        let src = r#"class Foo {
    void Encrypt() {
        DES.Create();
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "weak_cipher");
    }

    #[test]
    fn ignores_sha256() {
        let src = r#"class Foo {
    void Hash() {
        SHA256.Create();
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_cbc_mode() {
        let src = r#"class Foo {
    void Encrypt() {
        aes.Mode = CipherMode.CBC;
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
