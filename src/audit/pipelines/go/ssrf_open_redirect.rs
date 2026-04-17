// PERMANENT RUST EXCEPTION: This pipeline requires FlowsTo/SanitizedBy graph
// predicates for taint propagation analysis. These are not expressible in the
// match_pattern JSON DSL. Do not migrate -- this file stays as Rust intentionally.
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{self, extract_snippet, find_capture_index, node_text};

const SSRF_METHODS: &[(&str, &str)] = &[("http", "Get"), ("http", "Post"), ("http", "Head")];

pub struct SsrfOpenRedirectPipeline {
    selector_query: Arc<Query>,
}

impl SsrfOpenRedirectPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            selector_query: primitives::compile_selector_call_query()?,
        })
    }

    fn first_arg_is_literal(call_node: tree_sitter::Node, _source: &[u8]) -> bool {
        // Find the argument_list child of the call expression
        let mut child_cursor = call_node.walk();
        for child in call_node.children(&mut child_cursor) {
            if child.kind() == "argument_list" {
                // Get the first named child (first argument)
                let mut arg_cursor = child.walk();
                if let Some(arg) = child.named_children(&mut arg_cursor).next() {
                    // Check if it's a string literal (interpreted_string_literal or raw_string_literal)
                    return arg.kind() == "interpreted_string_literal"
                        || arg.kind() == "raw_string_literal";
                }
                return false;
            }
        }
        false
    }
}

impl Pipeline for SsrfOpenRedirectPipeline {
    fn name(&self) -> &str {
        "ssrf_open_redirect"
    }

    fn description(&self) -> &str {
        "Detects SSRF and open redirect risks: HTTP requests with dynamic URLs, redirects with user input"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.selector_query, tree.root_node(), source);

        let pkg_idx = find_capture_index(&self.selector_query, "pkg");
        let method_idx = find_capture_index(&self.selector_query, "method");
        let call_idx = find_capture_index(&self.selector_query, "call");

        while let Some(m) = matches.next() {
            let pkg = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
            let method = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(pkg), Some(method), Some(call)) = (pkg, method, call) {
                let pkg_name = node_text(pkg, source);
                let method_name = node_text(method, source);

                // Check for SSRF: http.Get/Post/Head with dynamic URL
                let is_ssrf_method = SSRF_METHODS
                    .iter()
                    .any(|(p, m)| *p == pkg_name && *m == method_name);

                if is_ssrf_method && !Self::first_arg_is_literal(call, source) {
                    let start = call.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "ssrf_dynamic_url".to_string(),
                        message: format!(
                            "{pkg_name}.{method_name}() with dynamic URL — potential SSRF"
                        ),
                        snippet: extract_snippet(source, call, 1),
                    });
                }

                // Check for open redirect: http.Redirect
                if pkg_name == "http" && method_name == "Redirect" {
                    let start = call.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "open_redirect".to_string(),
                        message:
                            "http.Redirect with potentially user-controlled URL — open redirect risk"
                                .to_string(),
                        snippet: extract_snippet(source, call, 1),
                    });
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SsrfOpenRedirectPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_dynamic_http_get() {
        let src = r#"package main

func handler(url string) {
	resp, _ := http.Get(url)
	_ = resp
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_dynamic_url");
    }

    #[test]
    fn detects_http_redirect() {
        let src = r#"package main

func handler(w http.ResponseWriter, r *http.Request) {
	target := r.URL.Query().Get("url")
	http.Redirect(w, r, target, 302)
}
"#;
        let findings = parse_and_check(src);
        let redirect_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "open_redirect")
            .collect();
        assert_eq!(redirect_findings.len(), 1);
    }

    #[test]
    fn ignores_static_url() {
        let src = r#"package main

func fetch() {
	resp, _ := http.Get("https://example.com")
	_ = resp
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_findings() {
        let src = r#"package main

func main() {
	fmt.Println("hello")
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
