use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_invocation_query, extract_snippet, find_capture_index, node_text};

const FILE_METHODS: &[&str] = &[
    "ReadAllText",
    "ReadAllBytes",
    "ReadAllLines",
    "WriteAllText",
    "WriteAllBytes",
    "WriteAllLines",
    "OpenRead",
    "OpenWrite",
    "Delete",
];

pub struct CSharpPathTraversalPipeline {
    invocation_query: Arc<Query>,
}

impl CSharpPathTraversalPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            invocation_query: compile_invocation_query()?,
        })
    }
}

impl Pipeline for CSharpPathTraversalPipeline {
    fn name(&self) -> &str {
        "csharp_path_traversal"
    }

    fn description(&self) -> &str {
        "Detects path traversal risks: File operations and Path.Combine with unvalidated input"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.invocation_query, tree.root_node(), source);

        let fn_idx = find_capture_index(&self.invocation_query, "fn_expr");
        let args_idx = find_capture_index(&self.invocation_query, "args");
        let inv_idx = find_capture_index(&self.invocation_query, "invocation");

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
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(fn_node), Some(args_node), Some(inv_node)) = (fn_node, args_node, inv_node)
            {
                let fn_text = node_text(fn_node, source);
                let args_text = node_text(args_node, source);

                // File.ReadAllText(base + param), etc.
                let is_file_method =
                    FILE_METHODS.iter().any(|m| fn_text.contains(m)) && fn_text.contains("File");
                if is_file_method && (args_text.contains('+') || contains_interpolation(args_node))
                {
                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unvalidated_file_path".to_string(),
                        message: format!(
                            "{fn_text}() with dynamic path — validate with GetFullPath and StartsWith to prevent path traversal"
                        ),
                        snippet: extract_snippet(source, inv_node, 1),
                    });
                }

                // Path.Combine(base, param) without validation
                if fn_text.contains("Path") && fn_text.contains("Combine") {
                    // Check if arguments include non-literal values
                    let has_variable_arg = args_node.named_child_count() > 0 && {
                        let mut has_non_literal = false;
                        for i in 0..args_node.named_child_count() {
                            if let Some(child) = args_node.named_child(i)
                                && child.kind() != "string_literal" {
                                    has_non_literal = true;
                                    break;
                                }
                        }
                        has_non_literal
                    };

                    if has_variable_arg {
                        // Check if GetFullPath+StartsWith is used nearby
                        let source_str = std::str::from_utf8(source).unwrap_or("");
                        if !source_str.contains("GetFullPath") || !source_str.contains("StartsWith")
                        {
                            let start = inv_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unvalidated_path_combine".to_string(),
                                message: "Path.Combine() with dynamic argument — validate with GetFullPath and StartsWith".to_string(),
                                snippet: extract_snippet(source, inv_node, 1),
                            });
                        }
                    }
                }
            }
        }

        findings
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
        let pipeline = CSharpPathTraversalPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_file_read_concat() {
        let src = r#"class Foo {
    void Read(string name) {
        File.ReadAllText("/uploads/" + name);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_file_path");
    }

    #[test]
    fn detects_path_combine_dynamic() {
        let src = r#"class Foo {
    void Read(string name) {
        Path.Combine("/uploads", name);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unvalidated_path_combine");
    }

    #[test]
    fn ignores_static_file_path() {
        let src = r#"class Foo {
    void Read() {
        File.ReadAllText("/etc/config.txt");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_file_ops() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
