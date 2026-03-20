use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_invocation_query, extract_snippet, find_capture_index, node_text};

const HTTP_METHODS: &[&str] = &[
    "GetAsync",
    "PostAsync",
    "PutAsync",
    "DeleteAsync",
    "GetStringAsync",
    "GetByteArrayAsync",
    "GetStreamAsync",
    "SendAsync",
];

pub struct CSharpSsrfPipeline {
    invocation_query: Arc<Query>,
}

impl CSharpSsrfPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            invocation_query: compile_invocation_query()?,
        })
    }
}

impl Pipeline for CSharpSsrfPipeline {
    fn name(&self) -> &str {
        "csharp_ssrf"
    }

    fn description(&self) -> &str {
        "Detects SSRF and open redirect risks: HttpClient with dynamic URL, WebRequest.Create, Redirect"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
        let args_idx = find_capture_index(&self.invocation_query, "args");
        let inv_idx = find_capture_index(&self.invocation_query, "invocation");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_idx)
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

            if let (Some(fn_node), Some(args_node), Some(inv_node)) = (fn_node, args_node, inv_node)
            {
                let fn_text = node_text(fn_node, source);

                // HttpClient.GetAsync(param), etc.
                let is_http = HTTP_METHODS.iter().any(|m| fn_text.contains(m));
                if is_http && !is_literal_arg(args_node, source) {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "ssrf_dynamic_url".to_string(),
                            message: format!(
                                "{fn_text}() with dynamic URL — validate host against allowlist to prevent SSRF"
                            ),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                }

                // WebRequest.Create(param)
                if fn_text.contains("WebRequest")
                    && fn_text.contains("Create")
                    && !is_literal_arg(args_node, source)
                {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "ssrf_dynamic_url".to_string(),
                        message:
                            "WebRequest.Create() with dynamic URL — validate host to prevent SSRF"
                                .to_string(),
                        snippet: extract_snippet(source, inv_node, 1),
                    });
                }

                // Redirect(param) without UriKind.Relative
                if fn_text.contains("Redirect")
                    && !fn_text.contains("Permanent")
                    && !is_literal_arg(args_node, source)
                {
                    let args_text = node_text(args_node, source);
                    if !args_text.contains("Relative") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "open_redirect".to_string(),
                                message: "Redirect() with dynamic URL — validate against allowlist to prevent open redirect".to_string(),
                                snippet: extract_snippet(source, inv_node, 1),
                            });
                    }
                }
            }
        }

        findings
    }
}

/// Check if the first argument in an argument_list is a string literal.
/// C# tree-sitter wraps arguments in `argument` nodes, so we drill through.
fn is_literal_arg(args_node: tree_sitter::Node, _source: &[u8]) -> bool {
    if let Some(first_arg) = args_node.named_child(0) {
        let node_to_check = if first_arg.kind() == "argument" {
            // Drill into the argument wrapper
            first_arg.named_child(0)
        } else {
            Some(first_arg)
        };
        if let Some(inner) = node_to_check {
            return inner.kind() == "string_literal";
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CSharpSsrfPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_http_client_dynamic_url() {
        let src = r#"class Foo {
    async void Fetch(string url) {
        client.GetAsync(url);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_dynamic_url");
    }

    #[test]
    fn detects_open_redirect() {
        let src = r#"class Foo {
    void Redir(string url) {
        Redirect(url);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "open_redirect");
    }

    #[test]
    fn ignores_static_url() {
        let src = r#"class Foo {
    async void Fetch() {
        client.GetAsync("https://example.com");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_http() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
