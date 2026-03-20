use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{self, extract_snippet, find_capture_index, node_text};

pub struct GoTypeConfusionPipeline {
    selector_query: Arc<Query>,
    assertion_query: Arc<Query>,
    conversion_query: Arc<Query>,
}

impl GoTypeConfusionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            selector_query: primitives::compile_selector_call_query()?,
            assertion_query: primitives::compile_type_assertion_query()?,
            conversion_query: primitives::compile_type_conversion_query()?,
        })
    }

    fn is_guarded_assertion(assertion_node: tree_sitter::Node) -> bool {
        // A type assertion is "guarded" when used like: val, ok := x.(Type)
        // This means the parent is a short_var_declaration with 2 LHS identifiers
        if let Some(parent) = assertion_node.parent() {
            // The assertion may be inside an expression_list on the RHS
            let decl_node = if parent.kind() == "expression_list" {
                parent.parent()
            } else {
                Some(parent)
            };

            if let Some(decl) = decl_node
                && decl.kind() == "short_var_declaration" {
                    // Check if the LHS has 2 identifiers
                    if let Some(lhs) = decl.child_by_field_name("left") {
                        let mut count = 0;
                        let mut child_cursor = lhs.walk();
                        for child in lhs.children(&mut child_cursor) {
                            if child.is_named() {
                                count += 1;
                            }
                        }
                        return count >= 2;
                    }
                }
        }
        false
    }
}

impl Pipeline for GoTypeConfusionPipeline {
    fn name(&self) -> &str {
        "type_confusion"
    }

    fn description(&self) -> &str {
        "Detects type confusion risks: unsafe.Pointer casts, unguarded type assertions, uintptr arithmetic"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // 1. unsafe.Pointer casts
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.selector_query, tree.root_node(), source);
        let pkg_idx = find_capture_index(&self.selector_query, "pkg");
        let method_idx = find_capture_index(&self.selector_query, "method");
        let call_idx = find_capture_index(&self.selector_query, "call");

        while let Some(m) = matches.next() {
            let pkg = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
            let method = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(pkg), Some(method), Some(call)) = (pkg, method, call) {
                let pkg_name = node_text(pkg, source);
                let method_name = node_text(method, source);

                if pkg_name == "unsafe" && method_name == "Pointer" {
                    let start = call.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unsafe_pointer_cast".to_string(),
                        message: "unsafe.Pointer cast bypasses type safety — review carefully"
                            .to_string(),
                        snippet: extract_snippet(source, call, 1),
                    });
                }
            }
        }

        // 2. Unguarded type assertions
        let mut cursor2 = QueryCursor::new();
        let mut matches2 = cursor2.matches(&self.assertion_query, tree.root_node(), source);
        let assertion_idx = find_capture_index(&self.assertion_query, "type_assertion");

        while let Some(m) = matches2.next() {
            let assertion_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == assertion_idx)
                .map(|c| c.node);

            if let Some(assertion_node) = assertion_node {
                if Self::is_guarded_assertion(assertion_node) {
                    continue;
                }

                let start = assertion_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unguarded_type_assertion".to_string(),
                    message: "type assertion without comma-ok check may panic at runtime"
                        .to_string(),
                    snippet: extract_snippet(source, assertion_node, 1),
                });
            }
        }

        // 3. uintptr conversions
        let mut cursor3 = QueryCursor::new();
        let mut matches3 = cursor3.matches(&self.conversion_query, tree.root_node(), source);
        let type_name_idx = find_capture_index(&self.conversion_query, "type_name");
        let conversion_idx = find_capture_index(&self.conversion_query, "conversion");

        while let Some(m) = matches3.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_name_idx)
                .map(|c| c.node);
            let conversion_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == conversion_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(conversion_node)) = (type_node, conversion_node) {
                let type_name = node_text(type_node, source);

                if type_name == "uintptr" {
                    let start = conversion_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "uintptr_arithmetic".to_string(),
                        message: "uintptr conversion may allow unsafe pointer arithmetic"
                            .to_string(),
                        snippet: extract_snippet(source, conversion_node, 1),
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
        let pipeline = GoTypeConfusionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_unsafe_pointer() {
        let src = r#"package main

import "unsafe"

func f() {
	p := unsafe.Pointer(&x)
	_ = p
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_pointer_cast");
    }

    #[test]
    fn detects_unguarded_assertion() {
        let src = r#"package main

func f(x interface{}) {
	v := x.(string)
	_ = v
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unguarded_type_assertion");
    }

    #[test]
    fn ignores_guarded_assertion() {
        let src = r#"package main

func f(x interface{}) {
	v, ok := x.(string)
	_ = v
	_ = ok
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_uintptr() {
        let src = r#"package main

import "unsafe"

func f() {
	p := uintptr(unsafe.Pointer(&x))
	_ = p
}
"#;
        let findings = parse_and_check(src);
        // Should detect both unsafe.Pointer and uintptr
        let uintptr_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "uintptr_arithmetic")
            .collect();
        assert_eq!(uintptr_findings.len(), 1);
    }
}
