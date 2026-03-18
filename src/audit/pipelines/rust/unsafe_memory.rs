use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use super::primitives::{self, extract_snippet, find_capture_index, node_text};

pub struct UnsafeMemoryPipeline {
    unsafe_query: Arc<Query>,
    method_query: Arc<Query>,
}

impl UnsafeMemoryPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            unsafe_query: primitives::compile_unsafe_block_query()?,
            method_query: primitives::compile_method_call_query()?,
        })
    }
}

impl Pipeline for UnsafeMemoryPipeline {
    fn name(&self) -> &str {
        "unsafe_memory"
    }

    fn description(&self) -> &str {
        "Detects unsafe memory operations: raw pointer dereference, pointer arithmetic, transmute in unsafe blocks"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.unsafe_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.unsafe_query, "unsafe_body");
        let block_idx = find_capture_index(&self.unsafe_query, "unsafe_block");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let block_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == block_idx)
                .map(|c| c.node);

            if let (Some(body), Some(block)) = (body_node, block_node) {
                // Check for transmute calls within this unsafe block
                // Walk descendant nodes to find call_expression whose function
                // text contains "transmute" — handles both plain and turbofish syntax
                {
                    let mut stack = vec![body];
                    while let Some(node) = stack.pop() {
                        if node.kind() == "call_expression" {
                            if let Some(func) = node.child_by_field_name("function") {
                                let func_text = node_text(func, source);
                                if func_text.contains("transmute") {
                                    let start = node.start_position();
                                    findings.push(AuditFinding {
                                        file_path: file_path.to_string(),
                                        line: start.row as u32 + 1,
                                        column: start.column as u32 + 1,
                                        severity: "error".to_string(),
                                        pipeline: self.name().to_string(),
                                        pattern: "transmute_in_unsafe".to_string(),
                                        message:
                                            "mem::transmute in unsafe block bypasses type safety"
                                                .to_string(),
                                        snippet: extract_snippet(source, block, 3),
                                    });
                                    // Don't recurse into this call's children
                                    continue;
                                }
                            }
                        }
                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                stack.push(child);
                            }
                        }
                    }
                }

                // Check for pointer arithmetic method calls (.offset(), .add(), .sub())
                {
                    let mut inner_cursor = QueryCursor::new();
                    inner_cursor.set_byte_range(body.byte_range());
                    let mut inner_matches =
                        inner_cursor.matches(&self.method_query, tree.root_node(), source);
                    let name_idx = find_capture_index(&self.method_query, "method_name");

                    // Non-pointer receiver patterns: calls like Duration.add(), Instant.sub()
                    const NON_POINTER_PATTERNS: &[&str] = &[
                        "Duration", "duration", "chrono", "time", "Instant", "instant",
                        "DateTime", "NaiveDate", "NaiveTime", "NaiveDateTime",
                    ];

                    while let Some(im) = inner_matches.next() {
                        let name_node = im
                            .captures
                            .iter()
                            .find(|c| c.index as usize == name_idx)
                            .map(|c| c.node);
                        let call_node = im
                            .captures
                            .iter()
                            .find(|c| c.index as usize == find_capture_index(&self.method_query, "call"))
                            .map(|c| c.node);
                        if let Some(name_node) = name_node {
                            let method_name = node_text(name_node, source);
                            if matches!(method_name, "offset" | "add" | "sub") {
                                // For .add() and .sub(), verify receiver looks like a pointer
                                if matches!(method_name, "add" | "sub") {
                                    if let Some(call) = call_node {
                                        // The call_expression has a function field which is a
                                        // field_expression; the object of that field_expression
                                        // is the receiver.
                                        if let Some(func) = call.child_by_field_name("function") {
                                            if let Some(obj) = func.child_by_field_name("value") {
                                                let receiver_text = node_text(obj, source);
                                                // Skip if receiver matches non-pointer patterns
                                                if NON_POINTER_PATTERNS.iter().any(|p| receiver_text.contains(p)) {
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                }

                                let start = name_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "pointer_arithmetic".to_string(),
                                    message: format!(
                                        "pointer arithmetic via .{}() in unsafe block",
                                        method_name
                                    ),
                                    snippet: extract_snippet(source, block, 3),
                                });
                            }
                        }
                    }
                }

                // Check for raw pointer dereference (*ptr) within the unsafe body
                // In tree-sitter Rust, pointer dereference is a unary_expression with * operator
                {
                    let body_text = node_text(body, source);
                    if body_text.contains('*') {
                        let mut stack = vec![body];
                        while let Some(node) = stack.pop() {
                            if node.kind() == "unary_expression" {
                                if let Some(op) = node.child(0) {
                                    if node_text(op, source) == "*" {
                                        let start = node.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "error".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "raw_pointer_deref".to_string(),
                                            message:
                                                "raw pointer dereference in unsafe block"
                                                    .to_string(),
                                            snippet: extract_snippet(source, block, 3),
                                        });
                                    }
                                }
                            }
                            for i in 0..node.child_count() {
                                if let Some(child) = node.child(i) {
                                    stack.push(child);
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UnsafeMemoryPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_transmute() {
        let src = r#"
fn f() {
    unsafe { std::mem::transmute::<u32, f32>(42) };
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "transmute_in_unsafe");
    }

    #[test]
    fn detects_pointer_deref() {
        let src = r#"
fn f(p: *const i32) {
    unsafe { let x = *p; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_pointer_deref");
    }

    #[test]
    fn detects_pointer_arithmetic() {
        let src = r#"
fn f(p: *const i32) {
    unsafe { let q = p.offset(1); }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "pointer_arithmetic");
    }

    #[test]
    fn ignores_safe_code() {
        let src = r#"
fn f() {
    let x = &42;
    let y = *x;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
