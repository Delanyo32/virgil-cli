use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_direct_call_query, compile_method_call_security_query, compile_new_expression_query,
    extract_snippet, find_capture_index, is_safe_literal, node_text,
};

pub struct CodeInjectionPipeline {
    direct_call_query: Arc<Query>,
    new_expr_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl CodeInjectionPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            direct_call_query: compile_direct_call_query(language)?,
            new_expr_query: compile_new_expression_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

impl Pipeline for CodeInjectionPipeline {
    fn name(&self) -> &str {
        "code_injection"
    }

    fn description(&self) -> &str {
        "Detects code injection: eval(), new Function(), setTimeout/setInterval with string, vm.runInNewContext/runInContext"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // eval(), setTimeout(string), setInterval(string)
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

                    // eval(x) where x is not a string literal
                    if fn_name == "eval"
                        && let Some(first_arg) = args.named_child(0)
                        && !is_safe_literal(first_arg, source)
                    {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "eval_injection".to_string(),
                            message: "`eval()` with dynamic argument — code injection risk"
                                .to_string(),
                            snippet: extract_snippet(source, call, 1),
                        });
                    }

                    // setTimeout/setInterval with string first arg (not a function)
                    if (fn_name == "setTimeout" || fn_name == "setInterval")
                        && args.named_child_count() >= 2
                        && let Some(first_arg) = args.named_child(0)
                    {
                        let kind = first_arg.kind();
                        // Flag if first arg is a string (acts like eval)
                        if kind == "string" || kind == "template_string" {
                            let start = call.start_position();
                            findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "setTimeout_string_eval".to_string(),
                                    message: format!(
                                        "`{}()` with string argument — implicit eval, use a function instead",
                                        fn_name
                                    ),
                                    snippet: extract_snippet(source, call, 1),
                                });
                        }
                    }
                }
            }
        }

        // new Function(...)
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

                if let (Some(ctor), Some(args), Some(expr)) = (ctor_node, args_node, expr_node)
                    && node_text(ctor, source) == "Function"
                {
                    // Last arg is the function body — flag if not a literal
                    let arg_count = args.named_child_count();
                    if arg_count > 0
                        && let Some(last_arg) = args.named_child(arg_count - 1)
                        && !is_safe_literal(last_arg, source)
                    {
                        let start = expr.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "function_constructor_injection".to_string(),
                            message: "`new Function()` with dynamic body — code injection risk"
                                .to_string(),
                            snippet: extract_snippet(source, expr, 1),
                        });
                    }
                }
            }
        }

        // vm.runInNewContext / vm.runInContext
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

                    if obj_name == "vm"
                        && (method_name == "runInNewContext"
                            || method_name == "runInContext"
                            || method_name == "runInThisContext")
                        && let Some(first_arg) = args.named_child(0)
                        && !is_safe_literal(first_arg, source)
                    {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "vm_code_execution".to_string(),
                            message: format!(
                                "`vm.{}()` with dynamic code — code injection risk",
                                method_name
                            ),
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
        let pipeline = CodeInjectionPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_eval_with_variable() {
        let src = "eval(userInput);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "eval_injection");
    }

    #[test]
    fn ignores_eval_with_literal() {
        let src = r#"eval("1 + 2");"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_new_function_with_variable() {
        let src = "const fn = new Function(body);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "function_constructor_injection");
    }

    #[test]
    fn detects_settimeout_with_string() {
        let src = r#"setTimeout("alert('hi')", 1000);"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "setTimeout_string_eval");
    }

    #[test]
    fn ignores_settimeout_with_function() {
        let src = "setTimeout(() => {}, 1000);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_vm_run_in_new_context() {
        let src = "vm.runInNewContext(code, sandbox);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "vm_code_execution");
    }
}
