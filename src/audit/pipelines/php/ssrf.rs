use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_call_query, extract_snippet, find_capture_index, node_text,
};

const SSRF_FUNCTIONS: &[(&str, &str)] = &[
    ("file_get_contents", "ssrf_file_get_contents"),
    ("fopen", "ssrf_fopen"),
    ("copy", "ssrf_copy"),
    ("readfile", "ssrf_readfile"),
];

pub struct SsrfPipeline {
    call_query: Arc<Query>,
}

impl SsrfPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_function_call_query()?,
        })
    }
}

impl Pipeline for SsrfPipeline {
    fn name(&self) -> &str {
        "ssrf"
    }

    fn description(&self) -> &str {
        "Detects SSRF risks: file_get_contents, fopen, copy with dynamic URLs"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
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

            if let (Some(name_node), Some(args_node), Some(call_node)) =
                (name_node, args_node, call_node)
            {
                let fn_name = node_text(name_node, source);

                let matching = SSRF_FUNCTIONS.iter().find(|(name, _)| *name == fn_name);
                if matching.is_none() {
                    continue;
                }
                let (_, pattern) = matching.unwrap();

                // Check if the first argument is a static string literal
                if is_static_php_arg(args_node) {
                    continue;
                }

                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message: format!("`{fn_name}()` with dynamic URL/path — potential SSRF"),
                    snippet: extract_snippet(source, call_node, 1),
                });
            }
        }

        findings
    }
}

/// Checks if the first argument in a PHP arguments node is a static string literal.
fn is_static_php_arg(args_node: tree_sitter::Node) -> bool {
    if let Some(arg_wrapper) = args_node.named_child(0) {
        let expr = if arg_wrapper.kind() == "argument" {
            arg_wrapper.named_child(0)
        } else {
            Some(arg_wrapper)
        };
        if let Some(expr) = expr {
            return expr.kind() == "string" || expr.kind() == "encapsed_string";
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SsrfPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_file_get_contents_variable() {
        let src = "<?php\nfile_get_contents($url);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_file_get_contents");
    }

    #[test]
    fn detects_fopen_variable() {
        let src = "<?php\nfopen($path, 'r');\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ssrf_fopen");
    }

    #[test]
    fn ignores_static_path() {
        let src = "<?php\nfile_get_contents('config.json');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_unrelated_function() {
        let src = "<?php\nstrlen($input);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
