use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_direct_call_query, compile_method_call_security_query, compile_new_expression_query,
    extract_snippet, find_capture_index, node_text,
};

pub struct InsecureDeserializationPipeline {
    direct_call_query: Arc<Query>,
    new_expr_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl InsecureDeserializationPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            direct_call_query: compile_direct_call_query(language)?,
            new_expr_query: compile_new_expression_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

impl Pipeline for InsecureDeserializationPipeline {
    fn name(&self) -> &str {
        "insecure_deserialization"
    }

    fn description(&self) -> &str {
        "Detects insecure deserialization: eval/Function on parsed data, postMessage without origin check"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // eval(x) where x traces to JSON.parse
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.direct_call_query, tree.root_node(), source);
            let fn_idx = find_capture_index(&self.direct_call_query, "fn_name");
            let call_idx = find_capture_index(&self.direct_call_query, "call");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_n), Some(call)) = (fn_node, call_node) {
                    let fn_name = node_text(fn_n, source);
                    if fn_name == "eval" {
                        let call_text = node_text(call, source);
                        if call_text.contains("JSON.parse") {
                            let start = call.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "eval_parsed_data".to_string(),
                                message: "`eval()` on JSON.parse result — insecure deserialization"
                                    .to_string(),
                                snippet: extract_snippet(source, call, 1),
                            });
                        }
                    }
                }
            }
        }

        // new Function(x) where x contains JSON.parse
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.new_expr_query, tree.root_node(), source);
            let ctor_idx = find_capture_index(&self.new_expr_query, "constructor");
            let expr_idx = find_capture_index(&self.new_expr_query, "new_expr");

            while let Some(m) = matches.next() {
                let ctor_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == ctor_idx)
                    .map(|c| c.node);
                let expr_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == expr_idx)
                    .map(|c| c.node);

                if let (Some(ctor), Some(expr)) = (ctor_node, expr_node)
                    && node_text(ctor, source) == "Function" {
                        let expr_text = node_text(expr, source);
                        if expr_text.contains("JSON.parse") {
                            let start = expr.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "function_from_parsed_data".to_string(),
                                message:
                                    "`new Function()` from parsed data — insecure deserialization"
                                        .to_string(),
                                snippet: extract_snippet(source, expr, 1),
                            });
                        }
                    }
            }
        }

        // addEventListener('message', callback) without origin check
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
            let method_idx = find_capture_index(&self.method_call_query, "method");
            let args_idx = find_capture_index(&self.method_call_query, "args");
            let call_idx = find_capture_index(&self.method_call_query, "call");

            while let Some(m) = matches.next() {
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

                if let (Some(method), Some(args), Some(call)) = (method_node, args_node, call_node)
                {
                    let method_name = node_text(method, source);
                    if method_name == "addEventListener"
                        && let Some(first_arg) = args.named_child(0) {
                            let event_name = node_text(first_arg, source);
                            if event_name.contains("message") {
                                // Check if callback contains origin check
                                if let Some(callback) = args.named_child(1) {
                                    let cb_text = node_text(callback, source);
                                    if !cb_text.contains("origin") && !cb_text.contains(".source") {
                                        let start = call.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "warning".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "postmessage_no_origin_check".to_string(),
                                            message: "Message event listener without origin check — verify sender identity".to_string(),
                                            snippet: extract_snippet(source, call, 1),
                                        });
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
        let pipeline = InsecureDeserializationPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_eval_json_parse() {
        let src = "eval(JSON.parse(data));";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "eval_parsed_data");
    }

    #[test]
    fn detects_new_function_json_parse() {
        let src = "const fn = new Function(JSON.parse(body));";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "function_from_parsed_data");
    }

    #[test]
    fn detects_postmessage_no_origin() {
        let src = r#"window.addEventListener("message", (e) => { doStuff(e.data); });"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "postmessage_no_origin_check");
    }

    #[test]
    fn ignores_postmessage_with_origin_check() {
        let src = r#"window.addEventListener("message", (e) => { if (e.origin !== "https://safe.com") return; });"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
