use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_definition_query, compile_new_expression_query, extract_snippet,
    find_capture_index, node_text,
};

pub struct CppIntegerOverflowPipeline {
    new_query: Arc<Query>,
    fn_query: Arc<Query>,
}

impl CppIntegerOverflowPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            new_query: compile_new_expression_query()?,
            fn_query: compile_function_definition_query()?,
        })
    }

    fn walk_for_signed_to_size_t(
        node: tree_sitter::Node,
        source: &[u8],
        findings: &mut Vec<AuditFinding>,
        file_path: &str,
        pipeline_name: &str,
    ) {
        // Check for static_cast<size_t>(...) or static_cast<std::size_t>(...)
        if node.kind() == "call_expression" {
            if let Some(func) = node.child_by_field_name("function") {
                let func_text = node_text(func, source);
                if func_text.contains("static_cast<size_t>")
                    || func_text.contains("static_cast<std::size_t>")
                {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "signed_to_size_t".to_string(),
                        message: "signed-to-`size_t` cast — negative values wrap to large positive"
                            .to_string(),
                        snippet: extract_snippet(source, node, 1),
                    });
                    return;
                }
            }
        }

        // Check for C-style cast: (size_t)expr
        if node.kind() == "cast_expression" {
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_text = node_text(type_node, source);
                if type_text.contains("size_t") {
                    let start = node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: pipeline_name.to_string(),
                        pattern: "signed_to_size_t".to_string(),
                        message: "signed-to-`size_t` cast — negative values wrap to large positive"
                            .to_string(),
                        snippet: extract_snippet(source, node, 1),
                    });
                    return;
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_signed_to_size_t(child, source, findings, file_path, pipeline_name);
        }
    }

    fn walk_for_size_t_subtraction(
        node: tree_sitter::Node,
        source: &[u8],
        findings: &mut Vec<AuditFinding>,
        file_path: &str,
        pipeline_name: &str,
    ) {
        if node.kind() == "binary_expression" {
            // Check for "-" operator
            let full_text = node_text(node, source);
            if let Some(left) = node.child_by_field_name("left") {
                if let Some(right) = node.child_by_field_name("right") {
                    // Find the operator by checking children between left and right
                    let mut has_minus = false;
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "-" || node_text(child, source) == "-" {
                            has_minus = true;
                            break;
                        }
                    }
                    if !has_minus && !full_text.contains(" - ") {
                        // Not a subtraction
                    } else if has_minus || full_text.contains(" - ") {
                        // Check if both operands are identifiers with size-related names
                        let left_text = node_text(left, source);
                        let right_text = node_text(right, source);
                        let size_names = [
                            "size", "len", "length", "count", "offset", "pos", "idx", "index",
                            "capacity", "num",
                        ];
                        let left_is_size = left.kind() == "identifier"
                            && size_names.iter().any(|n| {
                                left_text == *n
                                    || left_text.ends_with("_size")
                                    || left_text.ends_with("_len")
                                    || left_text.ends_with("_count")
                                    || left_text.ends_with("_offset")
                                    || left_text.ends_with("_pos")
                            });
                        let right_is_size = right.kind() == "identifier"
                            && size_names.iter().any(|n| {
                                right_text == *n
                                    || right_text.ends_with("_size")
                                    || right_text.ends_with("_len")
                                    || right_text.ends_with("_count")
                                    || right_text.ends_with("_offset")
                                    || right_text.ends_with("_pos")
                            });

                        if left_is_size && right_is_size {
                            let start = node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: pipeline_name.to_string(),
                                pattern: "size_t_subtraction".to_string(),
                                message: "`size_t` subtraction may underflow — check `a >= b` before `a - b`".to_string(),
                                snippet: extract_snippet(source, node, 1),
                            });
                            return; // don't recurse into this node
                        }
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_size_t_subtraction(child, source, findings, file_path, pipeline_name);
        }
    }
}

impl Pipeline for CppIntegerOverflowPipeline {
    fn name(&self) -> &str {
        "cpp_integer_overflow"
    }

    fn description(&self) -> &str {
        "Detects integer overflow risks: unchecked multiplication in new[], signed-to-size_t casts, size_t subtraction"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Pattern 1: new_array_unchecked_multiply — new T[a*b] without overflow check
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.new_query, tree.root_node(), source);
            let new_idx = find_capture_index(&self.new_query, "new_expr");

            while let Some(m) = matches.next() {
                let new_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == new_idx);

                if let Some(new_cap) = new_cap {
                    let text = node_text(new_cap.node, source);

                    // Check if the new expression has array sizing with multiplication
                    if text.contains('[') && text.contains('*') {
                        // Verify neither operand of the multiply is a constant
                        // Extract the part inside brackets
                        if let Some(bracket_start) = text.find('[') {
                            if let Some(bracket_end) = text.find(']') {
                                let inside = &text[bracket_start + 1..bracket_end];
                                // Check that there's a * and the operands aren't pure numeric constants
                                if inside.contains('*') {
                                    let parts: Vec<&str> =
                                        inside.split('*').map(|s| s.trim()).collect();
                                    let all_constant = parts
                                        .iter()
                                        .all(|p| p.chars().all(|c| c.is_ascii_digit()));
                                    if !all_constant {
                                        let start = new_cap.node.start_position();
                                        findings.push(AuditFinding {
                                            file_path: file_path.to_string(),
                                            line: start.row as u32 + 1,
                                            column: start.column as u32 + 1,
                                            severity: "warning".to_string(),
                                            pipeline: self.name().to_string(),
                                            pattern: "new_array_unchecked_multiply".to_string(),
                                            message: "`new T[a*b]` with unchecked multiplication — potential integer overflow leading to undersized allocation".to_string(),
                                            snippet: extract_snippet(source, new_cap.node, 1),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pattern 2 & 3: Walk function bodies for signed_to_size_t and size_t_subtraction
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);
            let fn_body_idx = find_capture_index(&self.fn_query, "fn_body");

            while let Some(m) = matches.next() {
                let fn_body_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_body_idx);

                if let Some(fn_body_cap) = fn_body_cap {
                    Self::walk_for_signed_to_size_t(
                        fn_body_cap.node,
                        source,
                        &mut findings,
                        file_path,
                        self.name(),
                    );
                    Self::walk_for_size_t_subtraction(
                        fn_body_cap.node,
                        source,
                        &mut findings,
                        file_path,
                        self.name(),
                    );
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CppIntegerOverflowPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_new_array_multiply() {
        let src = "void f(int w, int h) { auto p = new int[w * h]; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "new_array_unchecked_multiply");
    }

    #[test]
    fn ignores_new_constant() {
        let src = "void f() { auto p = new int[100]; }";
        let findings = parse_and_check(src);
        let array_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "new_array_unchecked_multiply")
            .collect();
        assert_eq!(array_findings.len(), 0);
    }

    #[test]
    fn detects_signed_to_size_t() {
        let src = "void f(int n) { size_t s = static_cast<size_t>(n); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "signed_to_size_t");
    }

    #[test]
    fn no_finding_for_constant_array() {
        let src = "void f() { auto p = new int[10 * 20]; }";
        let findings = parse_and_check(src);
        let array_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "new_array_unchecked_multiply")
            .collect();
        assert_eq!(array_findings.len(), 0);
    }

    #[test]
    fn metadata_correct() {
        let src = "void f(int w, int h) { auto p = new int[w * h]; }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "cpp_integer_overflow");
    }
}
