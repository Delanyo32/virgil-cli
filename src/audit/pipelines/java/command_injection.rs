use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_invocation_with_object_query, compile_object_creation_query,
    extract_snippet, find_capture_index, node_text,
};

pub struct CommandInjectionPipeline {
    method_query: Arc<Query>,
    creation_query: Arc<Query>,
}

impl CommandInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_invocation_with_object_query()?,
            creation_query: compile_object_creation_query()?,
        })
    }
}

impl Pipeline for CommandInjectionPipeline {
    fn name(&self) -> &str {
        "command_injection"
    }

    fn description(&self) -> &str {
        "Detects command injection risks: Runtime.exec() or ProcessBuilder with dynamic arguments"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_runtime_exec(tree, source, file_path, &mut findings);
        self.check_process_builder(tree, source, file_path, &mut findings);
        findings
    }
}

impl CommandInjectionPipeline {
    fn check_runtime_exec(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.method_query, "method_name");
        let args_idx = find_capture_index(&self.method_query, "args");
        let inv_idx = find_capture_index(&self.method_query, "invocation");

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx).map(|c| c.node);
            let args_node = m.captures.iter().find(|c| c.index as usize == args_idx).map(|c| c.node);
            let inv_node = m.captures.iter().find(|c| c.index as usize == inv_idx).map(|c| c.node);

            if let (Some(method_node), Some(args_node), Some(inv_node)) = (method_node, args_node, inv_node) {
                let method_name = node_text(method_node, source);
                if method_name != "exec" {
                    continue;
                }

                let inv_text = node_text(inv_node, source);
                if !inv_text.contains("getRuntime") && !inv_text.contains("Runtime") {
                    continue;
                }

                let args_text = node_text(args_node, source);
                if args_text.contains('+') {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "runtime_exec_injection".to_string(),
                        message: "Runtime.exec() with string concatenation enables command injection".to_string(),
                        snippet: extract_snippet(source, inv_node, 1),
                    });
                }
            }
        }
    }

    fn check_process_builder(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.creation_query, tree.root_node(), source);

        let type_idx = find_capture_index(&self.creation_query, "type_name");
        let args_idx = find_capture_index(&self.creation_query, "args");
        let creation_idx = find_capture_index(&self.creation_query, "creation");

        while let Some(m) = matches.next() {
            let type_node = m.captures.iter().find(|c| c.index as usize == type_idx).map(|c| c.node);
            let args_node = m.captures.iter().find(|c| c.index as usize == args_idx).map(|c| c.node);
            let creation_node = m.captures.iter().find(|c| c.index as usize == creation_idx).map(|c| c.node);

            if let (Some(type_node), Some(args_node), Some(creation_node)) = (type_node, args_node, creation_node) {
                let type_name = node_text(type_node, source);
                if type_name != "ProcessBuilder" {
                    continue;
                }

                let args_text = node_text(args_node, source);
                let has_shell = args_text.contains("\"bash\"") || args_text.contains("\"sh\"")
                    || args_text.contains("\"/bin/sh\"") || args_text.contains("\"/bin/bash\"")
                    || args_text.contains("\"cmd\"");

                if has_shell && args_text.contains('+') {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "processbuilder_shell_injection".to_string(),
                        message: "ProcessBuilder with shell and string concatenation enables command injection".to_string(),
                        snippet: extract_snippet(source, creation_node, 1),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CommandInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_runtime_exec_injection() {
        let src = r#"class Cmd {
    void run(String input) {
        Runtime.getRuntime().exec("sh -c " + input);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "runtime_exec_injection");
    }

    #[test]
    fn detects_processbuilder_injection() {
        let src = r#"class Cmd {
    void run(String input) {
        new ProcessBuilder("bash", "-c", "echo " + input);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "processbuilder_shell_injection");
    }

    #[test]
    fn ignores_static_command() {
        let src = r#"class Cmd {
    void run() {
        Runtime.getRuntime().exec("ls");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_exec() {
        let src = r#"class Foo {
    void bar() {
        System.out.println("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
