use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const SSRF_FUNCTIONS: &[&str] = &["urlopen", "urlretrieve"];

const SSRF_METHODS: &[(&str, &str)] = &[
    ("requests", "get"),
    ("requests", "post"),
    ("requests", "put"),
    ("requests", "delete"),
    ("requests", "head"),
    ("requests", "patch"),
    ("urllib", "urlopen"),
    ("httpx", "get"),
    ("httpx", "post"),
];

pub struct SsrfPipeline {
    call_query: Arc<Query>,
}

impl SsrfPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for SsrfPipeline {
    fn name(&self) -> &str {
        "ssrf"
    }

    fn description(&self) -> &str {
        "Detects SSRF risks: HTTP requests with dynamic URLs"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx)
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

            if let (Some(fn_node), Some(args_node), Some(call_node)) =
                (fn_node, args_node, call_node)
            {
                let is_ssrf = match fn_node.kind() {
                    "identifier" => {
                        let name = node_text(fn_node, source);
                        SSRF_FUNCTIONS.contains(&name)
                    }
                    "attribute" => {
                        let obj = fn_node
                            .child_by_field_name("object")
                            .map(|n| node_text(n, source));
                        let attr = fn_node
                            .child_by_field_name("attribute")
                            .map(|n| node_text(n, source));
                        if let (Some(obj_name), Some(attr_name)) = (obj, attr) {
                            SSRF_METHODS
                                .iter()
                                .any(|(m, f)| *m == obj_name && *f == attr_name)
                        } else {
                            false
                        }
                    }
                    _ => false,
                };

                if !is_ssrf {
                    continue;
                }

                // Check if first arg is a plain string literal (safe) or dynamic (potential SSRF)
                if let Some(first_arg) = args_node.named_child(0)
                    && first_arg.kind() == "string" && !has_interpolation(first_arg) {
                        continue;
                    }

                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "ssrf_dynamic_url".to_string(),
                    message: "HTTP request with dynamic URL — potential SSRF".to_string(),
                    snippet: extract_snippet(source, call_node, 1),
                });
            }
        }

        findings
    }
}

fn has_interpolation(node: tree_sitter::Node) -> bool {
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i)
            && child.kind() == "interpolation" {
                return true;
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SsrfPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_urlopen_with_variable() {
        let src = "from urllib.request import urlopen\nurlopen(url)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_dynamic_url");
    }

    #[test]
    fn detects_requests_get_dynamic() {
        let src = "import requests\nrequests.get(url)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn ignores_urlopen_with_literal() {
        let src = "urlopen(\"https://example.com\")";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_unrelated_call() {
        let src = "print(url)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
