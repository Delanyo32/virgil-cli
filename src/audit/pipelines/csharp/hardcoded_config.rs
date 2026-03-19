use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_string_literal_query, extract_snippet, find_capture_index, node_text,
};

const SUSPICIOUS_PATTERNS: &[(&str, &str)] = &[
    ("Server=", "connection string"),
    ("Data Source=", "connection string"),
    ("Password=", "password"),
    ("password=", "password"),
    ("Bearer ", "API token"),
    ("sk_", "secret key"),
    ("sk-", "secret key"),
    ("api_key", "API key"),
    ("apikey", "API key"),
    ("https://api.", "hardcoded API endpoint"),
    ("http://api.", "hardcoded API endpoint"),
    ("secret", "secret value"),
];

pub struct HardcodedConfigPipeline {
    string_query: Arc<Query>,
}

impl HardcodedConfigPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            string_query: compile_string_literal_query()?,
        })
    }
}

impl Pipeline for HardcodedConfigPipeline {
    fn name(&self) -> &str {
        "hardcoded_config"
    }

    fn description(&self) -> &str {
        "Detects hardcoded connection strings, API keys, and secrets in string literals"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.string_query, tree.root_node(), source);

        let str_lit_idx = find_capture_index(&self.string_query, "str_lit");

        while let Some(m) = matches.next() {
            let str_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == str_lit_idx)
                .map(|c| c.node);

            if let Some(str_node) = str_node {
                let text = node_text(str_node, source);
                // Strip outer quotes
                let inner = text.trim_matches('"').trim_matches('@');

                for &(pattern, kind) in SUSPICIOUS_PATTERNS {
                    if inner.contains(pattern) {
                        let start = str_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "hardcoded_config_value".to_string(),
                            message: format!(
                                "hardcoded {kind} detected \u{2014} use configuration/secrets management instead"
                            ),
                            snippet: extract_snippet(source, str_node, 3),
                        });
                        break; // one finding per string literal
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = HardcodedConfigPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_connection_string() {
        let src = r#"
class Config {
    string conn = "Server=localhost;Database=mydb;Password=secret123";
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "hardcoded_config_value");
    }

    #[test]
    fn detects_bearer_token() {
        let src = r#"
class Api {
    void Call() {
        var token = "Bearer eyJhbGciOiJIUzI1NiJ9";
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_secret_key() {
        let src = r#"
class Payment {
    string key = "sk_live_abc123";
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_normal_strings() {
        let src = r#"
class Foo {
    string name = "hello world";
    string label = "Submit";
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"class Foo { string s = "Password=admin"; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "hardcoded_config");
    }
}
