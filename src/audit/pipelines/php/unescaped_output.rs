use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::php_primitives::{
    compile_echo_statement_query, extract_snippet, find_capture_index, node_text,
};

const SUPERGLOBALS: &[&str] = &["$_GET", "$_POST", "$_REQUEST", "$_COOKIE", "$_SERVER"];

pub struct UnescapedOutputPipeline {
    echo_query: Arc<Query>,
}

impl UnescapedOutputPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            echo_query: compile_echo_statement_query()?,
        })
    }
}

impl Pipeline for UnescapedOutputPipeline {
    fn name(&self) -> &str {
        "unescaped_output"
    }

    fn description(&self) -> &str {
        "Detects echo statements outputting superglobals without HTML escaping"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.echo_query, tree.root_node(), source);

        let echo_idx = find_capture_index(&self.echo_query, "echo");

        while let Some(m) = matches.next() {
            let cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == echo_idx);

            if let Some(cap) = cap {
                let node = cap.node;

                // Walk descendants looking for superglobal variable names
                if let Some(superglobal) = find_unescaped_superglobal(node, source) {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "xss_superglobal".to_string(),
                        message: format!(
                            "echoing `{superglobal}` without escaping — wrap in htmlspecialchars() to prevent XSS"
                        ),
                        snippet: extract_snippet(source, node, 2),
                    });
                }
            }
        }

        findings
    }
}

fn find_unescaped_superglobal<'a>(
    node: tree_sitter::Node<'a>,
    source: &'a [u8],
) -> Option<&'a str> {
    // Walk the echo statement's descendants
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        // If we encounter a function call to an escaping function, skip its subtree
        if current.kind() == "function_call_expression" {
            if let Some(name_child) = current.child_by_field_name("function") {
                let fn_name = node_text(name_child, source);
                if fn_name == "htmlspecialchars"
                    || fn_name == "htmlentities"
                    || fn_name == "strip_tags"
                {
                    continue; // skip this subtree
                }
            }
        }

        if current.kind() == "variable_name" {
            let var_name = node_text(current, source);
            if SUPERGLOBALS.contains(&var_name) {
                return Some(var_name);
            }
        }

        for i in 0..current.named_child_count() {
            if let Some(child) = current.named_child(i) {
                stack.push(child);
            }
        }
    }
    None
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
        let pipeline = UnescapedOutputPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_raw_get() {
        let src = "<?php\necho $_GET['name'];\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "xss_superglobal");
        assert!(findings[0].message.contains("$_GET"));
    }

    #[test]
    fn detects_raw_post() {
        let src = "<?php\necho $_POST['data'];\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_escaped_output() {
        let src = "<?php\necho htmlspecialchars($_GET['name']);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_htmlentities() {
        let src = "<?php\necho htmlentities($_POST['data']);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_superglobal() {
        let src = "<?php\necho $name;\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
