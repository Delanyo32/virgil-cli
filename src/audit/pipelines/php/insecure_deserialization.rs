use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_call_query, extract_snippet, find_capture_index, node_text,
};

const SUPERGLOBALS: &[&str] = &["$_GET", "$_POST", "$_REQUEST", "$_COOKIE", "$_SERVER"];

pub struct InsecureDeserializationPipeline {
    call_query: Arc<Query>,
}

impl InsecureDeserializationPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_function_call_query()?,
        })
    }
}

impl Pipeline for InsecureDeserializationPipeline {
    fn name(&self) -> &str {
        "insecure_deserialization"
    }

    fn description(&self) -> &str {
        "Detects insecure deserialization: unserialize() with user input or without allowed_classes restriction"
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
                if fn_name != "unserialize" {
                    continue;
                }

                let call_text = node_text(call_node, source);

                // Check if the argument contains a superglobal (direct user input)
                let has_superglobal = SUPERGLOBALS.iter().any(|sg| call_text.contains(sg));

                if has_superglobal {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unserialize_user_input".to_string(),
                        message: "unserialize() with user input — potential remote code execution"
                            .to_string(),
                        snippet: extract_snippet(source, call_node, 1),
                    });
                    continue;
                }

                // Check if allowed_classes restriction is missing (only 1 arg)
                // PHP wraps args in `argument` nodes, so count those
                let arg_count = args_node
                    .named_children(&mut args_node.walk())
                    .filter(|c| c.kind() == "argument")
                    .count();
                if arg_count < 2 {
                    // Check the first argument isn't a string literal
                    let first_expr = args_node.named_child(0).and_then(|arg| {
                        if arg.kind() == "argument" {
                            arg.named_child(0)
                        } else {
                            Some(arg)
                        }
                    });
                    if let Some(expr) = first_expr {
                        if expr.kind() == "string" || expr.kind() == "encapsed_string" {
                            continue;
                        }
                    }

                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unserialize_no_restrict".to_string(),
                        message: "unserialize() without allowed_classes restriction — pass ['allowed_classes' => false] as second argument".to_string(),
                        snippet: extract_snippet(source, call_node, 1),
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = InsecureDeserializationPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_unserialize_cookie() {
        let src = "<?php\nunserialize($_COOKIE['session']);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unserialize_user_input");
    }

    #[test]
    fn detects_unserialize_get() {
        let src = "<?php\nunserialize($_GET['data']);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unserialize_user_input");
    }

    #[test]
    fn detects_unserialize_no_restriction() {
        let src = "<?php\nunserialize($data);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unserialize_no_restrict");
    }

    #[test]
    fn ignores_json_decode() {
        let src = "<?php\njson_decode($data);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
