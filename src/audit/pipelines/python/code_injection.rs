use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const DANGEROUS_BUILTINS: &[&str] = &["eval", "exec", "compile"];

pub struct CodeInjectionPipeline {
    call_query: Arc<Query>,
}

impl CodeInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for CodeInjectionPipeline {
    fn name(&self) -> &str {
        "code_injection"
    }

    fn description(&self) -> &str {
        "Detects code injection risks: eval/exec/compile with dynamic arguments"
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
                if fn_node.kind() != "identifier" {
                    continue;
                }
                let fn_name = node_text(fn_node, source);
                if !DANGEROUS_BUILTINS.contains(&fn_name) {
                    continue;
                }

                // Check if the first argument is a plain string literal (safe) or dynamic (unsafe)
                // f-strings (string with interpolation) are still dangerous
                if let Some(first_arg) = args_node.named_child(0) {
                    if first_arg.kind() == "string" && !has_interpolation(first_arg) {
                        continue;
                    }
                    if first_arg.kind() == "concatenated_string" {
                        continue;
                    }
                }

                let pattern = format!("{}_injection", fn_name);
                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "error".to_string(),
                    pipeline: self.name().to_string(),
                    pattern,
                    message: format!(
                        "`{fn_name}()` called with dynamic argument — potential code injection"
                    ),
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
            && child.kind() == "interpolation"
        {
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
        let pipeline = CodeInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_eval_with_variable() {
        let src = "eval(user_input)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "eval_injection");
    }

    #[test]
    fn detects_exec_with_variable() {
        let src = "exec(code)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "exec_injection");
    }

    #[test]
    fn ignores_eval_with_literal() {
        let src = "eval(\"1+1\")";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_safe_function() {
        let src = "print(user_input)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
