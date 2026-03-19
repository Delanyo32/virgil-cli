use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

pub struct CommandInjectionPipeline {
    call_query: Arc<Query>,
}

impl CommandInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl Pipeline for CommandInjectionPipeline {
    fn name(&self) -> &str {
        "command_injection"
    }

    fn description(&self) -> &str {
        "Detects command injection risks: os.system/popen, subprocess with shell=True and dynamic arguments"
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
                if fn_node.kind() == "attribute" {
                    let obj = fn_node
                        .child_by_field_name("object")
                        .map(|n| node_text(n, source));
                    let attr = fn_node
                        .child_by_field_name("attribute")
                        .map(|n| node_text(n, source));

                    match (obj, attr) {
                        // os.system() / os.popen() with non-literal arg
                        (Some("os"), Some("system")) | (Some("os"), Some("popen")) => {
                            if let Some(first_arg) = args_node.named_child(0) {
                                if first_arg.kind() != "string" {
                                    let start = call_node.start_position();
                                    findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "error".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "os_system_injection".to_string(),
                                        message: format!(
                                            "`os.{}()` with dynamic argument — use subprocess with list args instead",
                                            attr.unwrap_or("system")
                                        ),
                                        snippet: extract_snippet(source, call_node, 1),
                                    });
                                }
                            }
                        }
                        // subprocess.run/Popen/call with shell=True
                        (
                            Some("subprocess"),
                            Some("run" | "Popen" | "call" | "check_output" | "check_call"),
                        ) => {
                            let call_text = node_text(call_node, source);
                            if call_text.contains("shell=True")
                                || call_text.contains("shell = True")
                            {
                                // Check if first arg is a string (not a list)
                                if let Some(first_arg) = args_node.named_child(0) {
                                    if first_arg.kind() != "list" {
                                        let start = call_node.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "error".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "subprocess_shell_injection".to_string(),
                                            message: format!(
                                                "`subprocess.{}()` with shell=True and dynamic command — pass a list instead",
                                                attr.unwrap_or("run")
                                            ),
                                            snippet: extract_snippet(source, call_node, 1),
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
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
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CommandInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_os_system_with_variable() {
        let src = "import os\nos.system(\"ping \" + host)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "os_system_injection");
    }

    #[test]
    fn detects_subprocess_shell_true() {
        let src = "import subprocess\nsubprocess.run(cmd, shell=True)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "subprocess_shell_injection");
    }

    #[test]
    fn ignores_subprocess_with_list() {
        let src = "import subprocess\nsubprocess.run([\"ls\", \"-la\"])";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_os_system_with_literal() {
        let src = "import os\nos.system(\"ls -la\")";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
