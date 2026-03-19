use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_invocation_query, extract_snippet, find_capture_index, node_text};

pub struct ReflectionUnsafePipeline {
    invocation_query: Arc<Query>,
}

impl ReflectionUnsafePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            invocation_query: compile_invocation_query()?,
        })
    }
}

impl Pipeline for ReflectionUnsafePipeline {
    fn name(&self) -> &str {
        "reflection_unsafe"
    }

    fn description(&self) -> &str {
        "Detects unsafe reflection: Type.GetType, Activator.CreateInstance, Assembly.LoadFrom, unsafe pointer arithmetic"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_invocations(tree, source, file_path, &mut findings);
        self.check_unsafe_blocks(tree, source, file_path, &mut findings);
        findings
    }
}

impl ReflectionUnsafePipeline {
    fn check_invocations(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
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

                // Type.GetType(param) — unsafe dynamic type loading
                if fn_text.contains("Type") && fn_text.contains("GetType") {
                    if !is_literal_arg(args_node, source) {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unsafe_type_loading".to_string(),
                            message:
                                "Type.GetType() with dynamic input — validate against allowlist"
                                    .to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }

                // Activator.CreateInstance with Type.GetType nearby
                if fn_text.contains("Activator") && fn_text.contains("CreateInstance") {
                    if !is_literal_arg(args_node, source) {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unsafe_type_loading".to_string(),
                            message: "Activator.CreateInstance() with dynamic type — validate against allowlist".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }

                // Assembly.LoadFrom(param) — unsafe assembly loading
                if fn_text.contains("Assembly")
                    && (fn_text.contains("LoadFrom")
                        || fn_text.contains("LoadFile")
                        || fn_text.contains("UnsafeLoadFrom"))
                {
                    if !is_literal_arg(args_node, source) {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unsafe_assembly_load".to_string(),
                            message:
                                "Assembly loading with dynamic path — potential code execution"
                                    .to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }
            }
        }
    }
}

/// Check if the first argument in an argument_list is a string literal.
/// C# tree-sitter wraps arguments in `argument` nodes, so we drill through.
fn is_literal_arg(args_node: tree_sitter::Node, _source: &[u8]) -> bool {
    if let Some(first_arg) = args_node.named_child(0) {
        let node_to_check = if first_arg.kind() == "argument" {
            first_arg.named_child(0)
        } else {
            Some(first_arg)
        };
        if let Some(inner) = node_to_check {
            return inner.kind() == "string_literal";
        }
    }
    false
}

impl ReflectionUnsafePipeline {
    fn check_unsafe_blocks(
        &self,
        _tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let source_str = std::str::from_utf8(source).unwrap_or("");

        // Look for unsafe blocks with pointer arithmetic
        let mut in_unsafe = false;
        for (i, line) in source_str.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains("unsafe") && (trimmed.contains('{') || trimmed.ends_with("unsafe"))
            {
                in_unsafe = true;
            }
            if in_unsafe {
                // Check for pointer arithmetic patterns
                if trimmed.contains("->")
                    || (trimmed.contains('*') && trimmed.contains('(') && trimmed.contains("ptr"))
                    || trimmed.contains("stackalloc")
                {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: i as u32 + 1,
                        column: 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unsafe_pointer_arithmetic".to_string(),
                        message: "Unsafe pointer operation — potential memory corruption"
                            .to_string(),
                        snippet: trimmed.to_string(),
                    });
                    break;
                }
                if trimmed.contains('}') && !trimmed.contains('{') {
                    in_unsafe = false;
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ReflectionUnsafePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cs")
    }

    #[test]
    fn detects_dynamic_type_gettype() {
        let src = r#"class Foo {
    void Load(string typeName) {
        Type.GetType(typeName);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_type_loading");
    }

    #[test]
    fn detects_dynamic_assembly_load() {
        let src = r#"class Foo {
    void Load(string path) {
        Assembly.LoadFrom(path);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_assembly_load");
    }

    #[test]
    fn ignores_static_type_gettype() {
        let src = r#"class Foo {
    void Load() {
        Type.GetType("System.String");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_reflection() {
        let src = r#"class Foo {
    void Bar() {
        Console.WriteLine("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
