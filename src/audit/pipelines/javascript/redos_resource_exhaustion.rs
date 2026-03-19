use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_method_call_security_query, compile_new_expression_query, compile_regex_query,
    extract_snippet, find_capture_index, has_nested_quantifier, is_safe_literal, node_text,
};

pub struct RedosResourceExhaustionPipeline {
    regex_query: Arc<Query>,
    new_expr_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl RedosResourceExhaustionPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            regex_query: compile_regex_query(language)?,
            new_expr_query: compile_new_expression_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

impl Pipeline for RedosResourceExhaustionPipeline {
    fn name(&self) -> &str {
        "redos_resource_exhaustion"
    }

    fn description(&self) -> &str {
        "Detects ReDoS via nested quantifiers, dynamic RegExp, unbounded data accumulation"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Regex literals with nested quantifiers
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.regex_query, tree.root_node(), source);
            let regex_idx = find_capture_index(&self.regex_query, "regex");

            while let Some(m) = matches.next() {
                let regex_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == regex_idx)
                    .map(|c| c.node);

                if let Some(regex) = regex_node {
                    let text = node_text(regex, source);
                    if has_nested_quantifier(text) {
                        let start = regex.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "redos_nested_quantifier".to_string(),
                            message: "Regex with nested quantifiers — potential ReDoS".to_string(),
                            snippet: extract_snippet(source, regex, 1),
                        });
                    }
                }
            }
        }

        // new RegExp(x) where x is non-literal
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.new_expr_query, tree.root_node(), source);
            let ctor_idx = find_capture_index(&self.new_expr_query, "constructor");
            let args_idx = find_capture_index(&self.new_expr_query, "args");
            let expr_idx = find_capture_index(&self.new_expr_query, "new_expr");

            while let Some(m) = matches.next() {
                let ctor_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == ctor_idx)
                    .map(|c| c.node);
                let args_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == args_idx)
                    .map(|c| c.node);
                let expr_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == expr_idx)
                    .map(|c| c.node);

                if let (Some(ctor), Some(args), Some(expr)) = (ctor_node, args_node, expr_node) {
                    if node_text(ctor, source) == "RegExp" {
                        if let Some(first_arg) = args.named_child(0) {
                            if !is_safe_literal(first_arg, source) {
                                let start = expr.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "dynamic_regexp".to_string(),
                                    message: "`new RegExp()` with dynamic pattern — potential ReDoS if user-controlled".to_string(),
                                    snippet: extract_snippet(source, expr, 1),
                                });
                            }
                        }
                    }
                }
            }
        }

        // req.on('data', ...) without size check
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

                    // req.on('data', callback) pattern
                    if method_name == "on"
                        && (obj_name == "req" || obj_name == "request" || obj_name == "socket")
                    {
                        if let Some(first_arg) = args.named_child(0) {
                            let arg_text = node_text(first_arg, source);
                            if arg_text.contains("data") {
                                // Check if callback body contains length check
                                if let Some(callback) = args.named_child(1) {
                                    let cb_text = node_text(callback, source);
                                    if !cb_text.contains(".length")
                                        && !cb_text.contains("maxSize")
                                        && !cb_text.contains("MAX_SIZE")
                                        && !cb_text.contains("limit")
                                    {
                                        let start = call.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "warning".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "unbounded_data_accumulation".to_string(),
                                            message: "Data event handler without size limit — potential memory exhaustion".to_string(),
                                            snippet: extract_snippet(source, call, 1),
                                        });
                                    }
                                }
                            }
                        }
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
        let pipeline = RedosResourceExhaustionPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_nested_quantifier_regex() {
        let src = r#"const re = /(a+)+$/;"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "redos_nested_quantifier");
    }

    #[test]
    fn ignores_safe_regex() {
        let src = r#"const re = /^[a-z]+$/;"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_dynamic_regexp() {
        let src = "const re = new RegExp(userPattern);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "dynamic_regexp");
    }

    #[test]
    fn ignores_static_regexp() {
        let src = r#"const re = new RegExp("^abc$");"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_unbounded_data() {
        let src = r#"req.on("data", (chunk) => { body += chunk; });"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_data_accumulation");
    }

    #[test]
    fn ignores_data_with_length_check() {
        let src =
            r#"req.on("data", (chunk) => { if (body.length > MAX) return; body += chunk; });"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
