use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_direct_call_query, compile_method_call_security_query, extract_snippet,
    find_capture_index, is_safe_literal, node_text,
};

pub struct SsrfPipeline {
    direct_call_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl SsrfPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            direct_call_query: compile_direct_call_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

impl Pipeline for SsrfPipeline {
    fn name(&self) -> &str {
        "ssrf"
    }

    fn description(&self) -> &str {
        "Detects SSRF: fetch/http.get/https.get with dynamic URLs, open redirect via res.redirect"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // fetch(x) direct call
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.direct_call_query, tree.root_node(), source);
            let fn_idx = find_capture_index(&self.direct_call_query, "fn_name");
            let args_idx = find_capture_index(&self.direct_call_query, "args");
            let call_idx = find_capture_index(&self.direct_call_query, "call");

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
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_n), Some(args), Some(call)) = (fn_node, args_node, call_node) {
                    let fn_name = node_text(fn_n, source);
                    if fn_name == "fetch"
                        && let Some(first_arg) = args.named_child(0)
                        && !is_safe_literal(first_arg, source)
                    {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "ssrf_fetch".to_string(),
                            message: "`fetch()` with dynamic URL — potential SSRF".to_string(),
                            snippet: extract_snippet(source, call, 1),
                        });
                    }
                }
            }
        }

        // http.get/request, https.get/request, axios.get/post, res.redirect
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
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

                    // http.get/request, https.get/request, axios.get/post/put/delete
                    let is_http_call = matches!(obj_name, "http" | "https" | "axios")
                        && matches!(
                            method_name,
                            "get" | "post" | "put" | "delete" | "request" | "patch"
                        );

                    if is_http_call
                        && let Some(first_arg) = args.named_child(0)
                        && !is_safe_literal(first_arg, source)
                    {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "ssrf_http_request".to_string(),
                            message: format!(
                                "`{}.{}()` with dynamic URL — potential SSRF",
                                obj_name, method_name
                            ),
                            snippet: extract_snippet(source, call, 1),
                        });
                    }

                    // res.redirect with dynamic arg
                    if obj_name == "res"
                        && method_name == "redirect"
                        && let Some(first_arg) = args.named_child(0)
                        && !is_safe_literal(first_arg, source)
                    {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "open_redirect".to_string(),
                            message: "`res.redirect()` with dynamic URL — potential open redirect"
                                .to_string(),
                            snippet: extract_snippet(source, call, 1),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let lang = Language::JavaScript;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SsrfPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_fetch_with_variable() {
        let src = "fetch(url);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_fetch");
    }

    #[test]
    fn ignores_fetch_with_literal() {
        let src = r#"fetch("https://api.example.com/data");"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_http_get_with_variable() {
        let src = "http.get(url, callback);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_http_request");
    }

    #[test]
    fn detects_axios_post_with_variable() {
        let src = "axios.post(endpoint, data);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_http_request");
    }

    #[test]
    fn detects_open_redirect() {
        let src = "res.redirect(userUrl);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "open_redirect");
    }

    #[test]
    fn ignores_redirect_with_literal() {
        let src = r#"res.redirect("/dashboard");"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
