use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_interpolated_string_query, compile_string_literal_query, extract_snippet,
    find_capture_index, is_csharp_suppressed, node_text,
};

/// Patterns with associated kind label and severity tier.
/// Tier: "error" = credential/password, "warning" = connection string/key, "info" = endpoint
const SUSPICIOUS_PATTERNS: &[(&str, &str, &str)] = &[
    ("Password=", "password", "error"),
    ("password=", "password", "error"),
    ("Bearer ", "API token", "error"),
    ("sk_live_", "secret key", "error"),
    ("sk_test_", "secret key", "warning"),
    ("sk_", "secret key", "warning"),
    ("sk-", "secret key", "warning"),
    ("Server=", "connection string", "warning"),
    ("Data Source=", "connection string", "warning"),
    ("api_key", "API key", "warning"),
    ("apikey", "API key", "warning"),
    ("https://api.", "hardcoded API endpoint", "info"),
    ("http://api.", "hardcoded API endpoint", "info"),
    ("secret", "secret value", "warning"),
];

/// Variable names that suggest a config key name, not a value.
const CONFIG_KEY_SUFFIXES: &[&str] = &["Key", "Name", "Path", "Setting", "Header", "Prefix"];

pub struct HardcodedConfigPipeline {
    string_query: Arc<Query>,
    interpolated_query: Arc<Query>,
}

impl HardcodedConfigPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            string_query: compile_string_literal_query()?,
            interpolated_query: compile_interpolated_string_query()?,
        })
    }
}

/// Check if a string node is inside a logging/tracing call.
fn is_in_log_call(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "invocation_expression" {
            if let Some(fn_expr) = n.child_by_field_name("function") {
                let fn_text = fn_expr.utf8_text(source).unwrap_or("");
                let fn_lower = fn_text.to_lowercase();
                if fn_lower.contains("log")
                    || fn_lower.contains("trace")
                    || fn_lower.contains("debug")
                    || fn_lower.contains("writeline")
                    || fn_lower.contains("write(")
                {
                    return true;
                }
            }
        }
        current = n.parent();
    }
    false
}

/// Check if the variable/field being assigned to has a config-key-like name.
fn is_config_key_variable(node: tree_sitter::Node, source: &[u8]) -> bool {
    // Walk up to find the variable declarator or field declaration
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "variable_declarator" || n.kind() == "field_declaration" {
            if let Some(name_node) = n.child_by_field_name("name") {
                let name = name_node.utf8_text(source).unwrap_or("");
                return CONFIG_KEY_SUFFIXES.iter().any(|suffix| name.ends_with(suffix));
            }
        }
        if n.kind() == "property_declaration" {
            if let Some(name_node) = n.child_by_field_name("name") {
                let name = name_node.utf8_text(source).unwrap_or("");
                return CONFIG_KEY_SUFFIXES.iter().any(|suffix| name.ends_with(suffix));
            }
        }
        current = n.parent();
    }
    false
}

/// Check a string's inner text against suspicious patterns. Returns (kind, severity) if matched.
fn check_suspicious(inner: &str) -> Option<(&'static str, &'static str)> {
    for &(pattern, kind, severity) in SUSPICIOUS_PATTERNS {
        if inner.contains(pattern) {
            return Some((kind, severity));
        }
    }
    None
}

impl GraphPipeline for HardcodedConfigPipeline {
    fn name(&self) -> &str {
        "hardcoded_config"
    }

    fn description(&self) -> &str {
        "Detects hardcoded connection strings, API keys, and secrets in string literals"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check regular string literals
        self.check_string_literals(tree, source, file_path, &mut findings);

        // Check interpolated strings
        self.check_interpolated_strings(tree, source, file_path, &mut findings);

        findings
    }
}

impl HardcodedConfigPipeline {
    fn check_string_literals(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
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
                self.check_string_node(str_node, source, file_path, findings);
            }
        }
    }

    fn check_interpolated_strings(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.interpolated_query, tree.root_node(), source);
        let interp_idx = find_capture_index(&self.interpolated_query, "interp_str");

        while let Some(m) = matches.next() {
            let str_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == interp_idx)
                .map(|c| c.node);

            if let Some(str_node) = str_node {
                self.check_string_node(str_node, source, file_path, findings);
            }
        }
    }

    fn check_string_node(
        &self,
        str_node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let text = node_text(str_node, source);
        // Strip outer quotes and prefixes (@, $)
        let inner = text
            .trim_start_matches('$')
            .trim_start_matches('@')
            .trim_matches('"');

        if let Some((kind, severity)) = check_suspicious(inner) {
            // Skip suppressed findings
            if is_csharp_suppressed(source, str_node, "hardcoded_config") {
                return;
            }

            // Skip strings inside logging calls
            if is_in_log_call(str_node, source) {
                return;
            }

            // Skip config key variable names (e.g., const string PasswordKey = "password")
            if is_config_key_variable(str_node, source) {
                return;
            }

            let start = str_node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: severity.to_string(),
                pipeline: self.name().to_string(),
                pattern: "hardcoded_config_value".to_string(),
                message: format!(
                    "hardcoded {kind} detected \u{2014} use configuration/secrets management instead"
                ),
                snippet: extract_snippet(source, str_node, 3),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "Config.cs")
    }

    fn parse_and_check_with_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = HardcodedConfigPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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
        // Password= is error severity
        assert_eq!(findings[0].severity, "error");
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
        assert_eq!(findings[0].severity, "error");
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
        assert_eq!(findings[0].severity, "error");
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
        assert_eq!(findings[0].pipeline, "hardcoded_config");
    }

    #[test]
    fn api_endpoint_is_info_severity() {
        let src = r#"
class Api {
    string url = "https://api.example.com/v1";
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn detects_interpolated_string() {
        let src = r#"
class Config {
    void M() {
        var conn = $"Server={host};Password=admin123";
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn config_key_variable_excluded() {
        let src = r#"
class Constants {
    const string PasswordKey = "password";
    const string ApiKeyName = "api_key";
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn excluded_in_test_files() {
        let src = r#"
class ConfigTests {
    void Test() {
        var conn = "Server=localhost;Password=test";
    }
}
"#;
        let findings = parse_and_check_with_path(src, "ConfigTests.cs");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppressed_by_nolint() {
        let src = r#"
class Config {
    // NOLINT
    string conn = "Server=localhost;Password=secret";
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
