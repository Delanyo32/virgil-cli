use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_assignment_expression_query, compile_object_creation_query, extract_snippet,
    find_capture_index, node_text,
};
use crate::audit::pipelines::helpers::{
    all_args_are_literals, is_literal_node_csharp, is_safe_expression,
};

pub struct CommandInjectionPipeline {
    creation_query: Arc<Query>,
    assign_query: Arc<Query>,
}

impl CommandInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            creation_query: compile_object_creation_query()?,
            assign_query: compile_assignment_expression_query()?,
        })
    }
}

impl Pipeline for CommandInjectionPipeline {
    fn name(&self) -> &str {
        "command_injection"
    }

    fn description(&self) -> &str {
        "Detects command injection risks: ProcessStartInfo with dynamic arguments, Process.Start with dynamic input"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_process_start_info(tree, source, file_path, &mut findings);
        self.check_argument_assignment(tree, source, file_path, &mut findings);
        findings
    }
}

impl CommandInjectionPipeline {
    fn check_process_start_info(
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
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let creation_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == creation_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(args_node), Some(creation_node)) =
                (type_node, args_node, creation_node)
            {
                let type_name = node_text(type_node, source);
                if type_name != "ProcessStartInfo" {
                    continue;
                }

                // Skip if all arguments are safe literals/constants
                if all_args_are_literals(args_node, is_literal_node_csharp) {
                    continue;
                }

                let args_text = node_text(args_node, source);
                let has_shell = args_text.contains("\"cmd\"")
                    || args_text.contains("\"bash\"")
                    || args_text.contains("\"sh\"")
                    || args_text.contains("\"powershell\"")
                    || args_text.contains("cmd.exe")
                    || args_text.contains("powershell.exe");

                if has_shell && (args_text.contains('+') || contains_interpolation(args_node)) {
                    let start = creation_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "shell_command_injection".to_string(),
                        message: "ProcessStartInfo with shell and dynamic arguments enables command injection".to_string(),
                        snippet: extract_snippet(source, creation_node, 2),
                    });
                }
            }
        }
    }

    fn check_argument_assignment(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.assign_query, tree.root_node(), source);

        let lhs_idx = find_capture_index(&self.assign_query, "lhs");
        let rhs_idx = find_capture_index(&self.assign_query, "rhs");
        let assign_idx = find_capture_index(&self.assign_query, "assign");

        while let Some(m) = matches.next() {
            let lhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == lhs_idx)
                .map(|c| c.node);
            let rhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == rhs_idx)
                .map(|c| c.node);
            let assign_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == assign_idx)
                .map(|c| c.node);

            if let (Some(lhs_node), Some(rhs_node), Some(assign_node)) =
                (lhs_node, rhs_node, assign_node)
            {
                let lhs_text = node_text(lhs_node, source);
                if !lhs_text.contains("Arguments") {
                    continue;
                }

                // Skip if the RHS is a safe literal expression
                if is_safe_expression(rhs_node, is_literal_node_csharp) {
                    continue;
                }

                let rhs_text = node_text(rhs_node, source);
                if rhs_text.contains('+') || rhs_node.kind() == "interpolated_string_expression" {
                    // Check if there's a shell process in context
                    let source_str = std::str::from_utf8(source).unwrap_or("");
                    let has_shell = source_str.contains("cmd")
                        || source_str.contains("bash")
                        || source_str.contains("powershell")
                        || source_str.contains("ProcessStartInfo");

                    if has_shell {
                        let start = assign_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "process_start_injection".to_string(),
                            message: "Process Arguments set with dynamic value — sanitize input to prevent command injection".to_string(),
                            snippet: extract_snippet(source, assign_node, 1),
                        });
                    }
                }
            }
        }
    }
}

fn contains_interpolation(node: tree_sitter::Node) -> bool {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind() == "interpolated_string_expression" {
            return true;
        }
        for i in 0..current.named_child_count() {
            if let Some(child) = current.named_child(i) {
                stack.push(child);
            }
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CommandInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_process_start_info_injection() {
        let src = r#"class Cmd {
    void Run(string input) {
        var psi = new ProcessStartInfo("cmd", "/c echo " + input);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "shell_command_injection");
    }

    #[test]
    fn ignores_static_command() {
        let src = r#"class Cmd {
    void Run() {
        var psi = new ProcessStartInfo("ls", "-la");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_process() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
