use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

const ALLOC_FUNCTIONS: &[&str] = &["malloc", "calloc", "realloc", "aligned_alloc"];
const MEMCPY_FAMILY: &[&str] = &["memcpy", "memmove", "strncpy"];

pub struct CIntegerOverflowPipeline {
    call_query: Arc<Query>,
}

impl CIntegerOverflowPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }

    /// Check if a node (or any descendant) contains a binary_expression with operator `*`.
    fn contains_unchecked_multiply(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "binary_expression" {
            // Find the operator by checking unnamed children between left and right
            let left = node.child_by_field_name("left");
            let right = node.child_by_field_name("right");

            if let (Some(left_node), Some(right_node)) = (left, right) {
                // The operator is between left and right positions
                let left_end = left_node.end_byte();
                let right_start = right_node.start_byte();
                if right_start > left_end {
                    let op_text = std::str::from_utf8(&source[left_end..right_start])
                        .unwrap_or("")
                        .trim();
                    if op_text == "*" {
                        // Check that neither side is a sizeof expression
                        let left_text = node_text(left_node, source);
                        let right_text = node_text(right_node, source);
                        if !left_text.starts_with("sizeof")
                            && !right_text.starts_with("sizeof")
                            && left_node.kind() != "sizeof_expression"
                            && right_node.kind() != "sizeof_expression"
                        {
                            return true;
                        }
                    }
                }
            }
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if Self::contains_unchecked_multiply(child, source) {
                return true;
            }
        }
        false
    }

    /// Walk up from a node to find the enclosing function_definition.
    fn find_enclosing_function(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        let mut current = node.parent();
        while let Some(n) = current {
            if n.kind() == "function_definition" {
                return Some(n);
            }
            current = n.parent();
        }
        None
    }

    /// Extract parameter names and their type text from a function_definition.
    fn extract_param_types(fn_def: tree_sitter::Node, source: &[u8]) -> Vec<(String, String)> {
        let mut params = Vec::new();
        if let Some(declarator) = fn_def.child_by_field_name("declarator")
            && let Some(param_list) = declarator.child_by_field_name("parameters") {
                let mut cursor = param_list.walk();
                for child in param_list.named_children(&mut cursor) {
                    if child.kind() == "parameter_declaration" {
                        let type_text = child
                            .child_by_field_name("type")
                            .map(|n| node_text(n, source).to_string())
                            .unwrap_or_default();
                        let name = child
                            .child_by_field_name("declarator")
                            .map(|n| Self::extract_identifier(n, source))
                            .unwrap_or_default();
                        if !name.is_empty() {
                            params.push((name, type_text));
                        }
                    }
                }
            }
        params
    }

    fn extract_identifier(node: tree_sitter::Node, source: &[u8]) -> String {
        if node.kind() == "identifier" {
            return node_text(node, source).to_string();
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::extract_identifier(inner, source);
        }
        String::new()
    }

    /// Check if a type string is a signed integer type (not size_t, not unsigned).
    fn is_signed_type(type_text: &str) -> bool {
        let t = type_text.trim();
        // These are unsigned/safe types
        if t.contains("size_t")
            || t.contains("unsigned")
            || t.contains("uint")
            || t.contains("uintptr")
        {
            return false;
        }
        // Common signed integer types
        t == "int"
            || t == "long"
            || t == "short"
            || t == "signed"
            || t.starts_with("int")
            || t.starts_with("long")
            || t.starts_with("short")
            || t.starts_with("signed")
            || t == "ssize_t"
            || t == "ptrdiff_t"
    }
}

impl Pipeline for CIntegerOverflowPipeline {
    fn name(&self) -> &str {
        "c_integer_overflow"
    }

    fn description(&self) -> &str {
        "Detects integer overflow risks: unchecked multiplication in malloc size, signed size in memcpy"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");
        let args_idx = find_capture_index(&self.call_query, "args");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);
            let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);

            if let (Some(fn_cap), Some(call_cap), Some(args_cap)) = (fn_cap, call_cap, args_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                let args_node = args_cap.node;
                let named_args: Vec<tree_sitter::Node> = {
                    let mut walker = args_node.walk();
                    args_node.named_children(&mut walker).collect()
                };

                // Pattern: malloc_unchecked_multiply
                if ALLOC_FUNCTIONS.contains(&fn_name) {
                    for arg in &named_args {
                        if Self::contains_unchecked_multiply(*arg, source) {
                            let start = call_cap.node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "malloc_unchecked_multiply".to_string(),
                                message: format!(
                                    "`{fn_name}()` with unchecked multiplication — potential integer overflow leading to undersized allocation"
                                ),
                                snippet: extract_snippet(source, call_cap.node, 1),
                            });
                            break;
                        }
                    }
                }

                // Pattern: memcpy_signed_size
                if MEMCPY_FAMILY.contains(&fn_name) {
                    // Size is the 3rd argument (index 2)
                    if let Some(size_arg) = named_args.get(2)
                        && size_arg.kind() == "identifier" {
                            let arg_name = node_text(*size_arg, source);

                            // Walk up to find enclosing function and check param type
                            if let Some(fn_def) = Self::find_enclosing_function(call_cap.node) {
                                let params = Self::extract_param_types(fn_def, source);
                                for (param_name, param_type) in &params {
                                    if param_name == arg_name && Self::is_signed_type(param_type) {
                                        let start = call_cap.node.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "warning".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "memcpy_signed_size".to_string(),
                                            message: format!(
                                                "`memcpy()` size from signed integer `{arg_name}` — negative values wrap to large positive, causing overflow"
                                            ),
                                            snippet: extract_snippet(
                                                source,
                                                call_cap.node,
                                                1,
                                            ),
                                        });
                                        break;
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
        let pipeline = CIntegerOverflowPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_malloc_multiply() {
        let src = "void f(int n, int m) { char *p = malloc(n * m); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "malloc_unchecked_multiply");
    }

    #[test]
    fn ignores_malloc_sizeof() {
        let src = "void f() { int *p = malloc(sizeof(int)); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_malloc_plain_multiply() {
        let src = "void f(size_t w, size_t h) { void *p = malloc(w * h); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "malloc_unchecked_multiply");
    }
}
