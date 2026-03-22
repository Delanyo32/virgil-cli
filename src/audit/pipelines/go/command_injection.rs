use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::{all_args_are_literals, is_literal_node_go};

use super::primitives::{
    compile_selector_call_query, extract_snippet, find_capture_index, node_text,
};

pub struct CommandInjectionPipeline {
    selector_query: Arc<Query>,
}

impl CommandInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            selector_query: compile_selector_call_query()?,
        })
    }
}

impl Pipeline for CommandInjectionPipeline {
    fn name(&self) -> &str {
        "command_injection"
    }

    fn description(&self) -> &str {
        "Detects command injection risks: exec.Command with shell invocation and dynamic arguments"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.selector_query, tree.root_node(), source);

        let pkg_idx = find_capture_index(&self.selector_query, "pkg");
        let method_idx = find_capture_index(&self.selector_query, "method");
        let call_idx = find_capture_index(&self.selector_query, "call");

        while let Some(m) = matches.next() {
            let pkg_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(pkg), Some(method), Some(call)) = (pkg_node, method_node, call_node) {
                let pkg_name = node_text(pkg, source);
                let method_name = node_text(method, source);

                if pkg_name != "exec" || method_name != "Command" {
                    continue;
                }

                let call_text = node_text(call, source);
                // Check if first arg is a shell
                let shells = [
                    "\"sh\"",
                    "\"bash\"",
                    "\"/bin/sh\"",
                    "\"/bin/bash\"",
                    "\"cmd\"",
                ];
                let is_shell = shells.iter().any(|s| call_text.contains(s));

                if !is_shell {
                    continue;
                }

                // Skip if all arguments are safe literals/constants
                let args_child = (0..call.child_count())
                    .filter_map(|i| call.child(i))
                    .find(|c| c.kind() == "argument_list");
                if let Some(args) = args_child
                    && all_args_are_literals(args, is_literal_node_go)
                {
                    continue;
                }

                // Check if there's string concatenation or variable in the arguments
                // (presence of + or a non-literal argument after "-c")
                let has_dynamic = call_text.contains('+')
                    || (call_text.contains("\"-c\"") && !call_text.ends_with("\")"));

                if has_dynamic {
                    let start = call.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "shell_command_injection".to_string(),
                        message: "exec.Command with shell invocation and dynamic arguments enables command injection".to_string(),
                        snippet: extract_snippet(source, call, 1),
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CommandInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_command_injection() {
        let src = r#"package main

import "os/exec"

func run(userInput string) {
	exec.Command("sh", "-c", userInput + " --flag")
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "shell_command_injection");
    }

    #[test]
    fn ignores_static_command() {
        let src = r#"package main

import "os/exec"

func run() {
	exec.Command("ls", "-la")
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_exec() {
        let src = r#"package main

import "fmt"

func run() {
	fmt.Println("hello")
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
