use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_direct_call_query, compile_method_call_security_query, extract_snippet,
    find_capture_index, is_safe_literal, node_text,
};

pub struct CommandInjectionPipeline {
    direct_call_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl CommandInjectionPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            direct_call_query: compile_direct_call_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

impl Pipeline for CommandInjectionPipeline {
    fn name(&self) -> &str {
        "command_injection"
    }

    fn description(&self) -> &str {
        "Detects command injection: exec/execSync with dynamic args, spawn with shell:true"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // child_process.exec/execSync/execFileSync with non-literal first arg
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

                    if matches!(method_name, "exec" | "execSync" | "execFileSync")
                        && let Some(first_arg) = args.named_child(0)
                            && !is_safe_literal(first_arg, source) {
                                let start = call.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "exec_command_injection".to_string(),
                                    message: format!(
                                        "`{}()` with dynamic argument — command injection risk",
                                        method_name
                                    ),
                                    snippet: extract_snippet(source, call, 1),
                                });
                            }

                    // spawn with shell: true
                    if method_name == "spawn" {
                        let call_text = node_text(call, source);
                        if call_text.contains("shell: true") || call_text.contains("shell:true") {
                            let start = call.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "spawn_shell_injection".to_string(),
                                message: "`spawn()` with `shell: true` — command injection risk"
                                    .to_string(),
                                snippet: extract_snippet(source, call, 1),
                            });
                        }
                    }
                }
            }
        }

        // Direct exec() calls (require('child_process').exec pattern)
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
                    if matches!(fn_name, "exec" | "execSync")
                        && let Some(first_arg) = args.named_child(0)
                            && !is_safe_literal(first_arg, source) {
                                let start = call.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "exec_command_injection".to_string(),
                                    message: format!(
                                        "`{}()` with dynamic argument — command injection risk",
                                        fn_name
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
        let pipeline = CommandInjectionPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_exec_with_variable() {
        let src = "child_process.exec(cmd);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "exec_command_injection");
    }

    #[test]
    fn detects_exec_sync_with_template() {
        let src = "child_process.execSync(`rm -rf ${dir}`);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "exec_command_injection");
    }

    #[test]
    fn detects_spawn_shell_true() {
        let src = r#"cp.spawn("cmd", args, { shell: true });"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "spawn_shell_injection");
    }

    #[test]
    fn ignores_exec_with_literal() {
        let src = r#"child_process.exec("ls -la");"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_spawn_without_shell() {
        let src = r#"cp.spawn("node", ["index.js"]);"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
