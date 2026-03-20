use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, compile_function_definition_query, extract_snippet,
    find_capture_index, node_text,
};

const COMMAND_FUNCTIONS: &[&str] = &["system", "popen"];

pub struct CCommandInjectionPipeline {
    call_query: Arc<Query>,
    fn_def_query: Arc<Query>,
}

impl CCommandInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
            fn_def_query: compile_function_definition_query()?,
        })
    }

    /// Walk function body looking for sprintf() filling a buffer that is later passed to system().
    fn find_sprintf_system_patterns(
        body: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut sprintf_buffers: Vec<(String, tree_sitter::Node)> = Vec::new();

        // Walk direct children of the compound_statement body
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            if child.kind() == "expression_statement" {
                // Look for call_expression inside
                let mut inner_cursor = child.walk();
                for inner in child.named_children(&mut inner_cursor) {
                    if inner.kind() == "call_expression"
                        && let Some(func) = inner.child_by_field_name("function") {
                            let fn_name = node_text(func, source);
                            if fn_name == "sprintf" {
                                // First arg to sprintf is the buffer
                                if let Some(args) = inner.child_by_field_name("arguments") {
                                    let mut args_cursor = args.walk();
                                    let named_args: Vec<tree_sitter::Node> =
                                        args.named_children(&mut args_cursor).collect();
                                    if let Some(buf_node) = named_args.first() {
                                        let buf_name = node_text(*buf_node, source).to_string();
                                        sprintf_buffers.push((buf_name, child));
                                    }
                                }
                            } else if fn_name == "system" {
                                // Check if first arg matches a known sprintf buffer
                                if let Some(args) = inner.child_by_field_name("arguments") {
                                    let mut args_cursor = args.walk();
                                    let named_args: Vec<tree_sitter::Node> =
                                        args.named_children(&mut args_cursor).collect();
                                    if let Some(arg_node) = named_args.first() {
                                        let arg_text = node_text(*arg_node, source);
                                        for (buf_name, _sprintf_node) in &sprintf_buffers {
                                            if arg_text == buf_name {
                                                let start = child.start_position();
                                                findings.push(AuditFinding {
                                                    file_path: file_path.to_string(),
                                                    line: start.row as u32 + 1,
                                                    column: start.column as u32 + 1,
                                                    severity: "error".to_string(),
                                                    pipeline: pipeline_name.to_string(),
                                                    pattern: "system_with_sprintf".to_string(),
                                                    message: format!(
                                                        "buffer `{buf_name}` filled by `sprintf()` then passed to `system()` — command injection risk"
                                                    ),
                                                    snippet: extract_snippet(source, child, 1),
                                                });
                                            }
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

impl Pipeline for CCommandInjectionPipeline {
    fn name(&self) -> &str {
        "c_command_injection"
    }

    fn description(&self) -> &str {
        "Detects command injection risks: system()/popen() with non-literal arguments"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Phase 1: Detect system()/popen() with dynamic arguments
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

            let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
            let call_idx = find_capture_index(&self.call_query, "call");
            let args_idx = find_capture_index(&self.call_query, "args");

            while let Some(m) = matches.next() {
                let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
                let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);
                let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);

                if let (Some(fn_cap), Some(call_cap), Some(args_cap)) = (fn_cap, call_cap, args_cap)
                {
                    let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                    if !COMMAND_FUNCTIONS.contains(&fn_name) {
                        continue;
                    }

                    // Get the first named argument
                    let args_node = args_cap.node;
                    let mut walker = args_node.walk();
                    let named_args: Vec<tree_sitter::Node> =
                        args_node.named_children(&mut walker).collect();

                    if let Some(first_arg) = named_args.first()
                        && first_arg.kind() != "string_literal"
                            && first_arg.kind() != "concatenated_string"
                        {
                            let pattern = if fn_name == "system" {
                                "system_dynamic_arg"
                            } else {
                                "popen_dynamic_arg"
                            };

                            let start = call_cap.node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: pattern.to_string(),
                                message: format!(
                                    "`{fn_name}()` called with dynamic argument — risk of command injection"
                                ),
                                snippet: extract_snippet(source, call_cap.node, 1),
                            });
                        }
                }
            }
        }

        // Phase 2: Detect sprintf() + system() pattern within function bodies
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

            let fn_body_idx = find_capture_index(&self.fn_def_query, "fn_body");

            while let Some(m) = matches.next() {
                let body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

                if let Some(body_cap) = body_cap {
                    let mut sprintf_system_findings = Self::find_sprintf_system_patterns(
                        body_cap.node,
                        source,
                        file_path,
                        self.name(),
                    );
                    findings.append(&mut sprintf_system_findings);
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CCommandInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_system_variable() {
        let src = "void f(char *cmd) { system(cmd); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "system_dynamic_arg");
        assert!(findings[0].message.contains("system"));
    }

    #[test]
    fn ignores_system_literal() {
        let src = r#"void f() { system("ls -la"); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_popen_variable() {
        let src = r#"void f(char *cmd) { popen(cmd, "r"); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "popen_dynamic_arg");
        assert!(findings[0].message.contains("popen"));
    }
}
