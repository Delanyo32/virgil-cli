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

pub struct CppExceptionSafetyPipeline {
    new_query: Arc<Query>,
    fn_query: Arc<Query>,
}

impl CppExceptionSafetyPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            new_query: compile_new_expression_query()?,
            fn_query: compile_function_definition_query()?,
        })
    }

    fn is_smart_ptr_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        if let Some(parent) = node.parent() {
            // Case: new expression inside argument_list
            if parent.kind() == "argument_list"
                && let Some(grandparent) = parent.parent() {
                    // call_expression: e.g., std::unique_ptr<int>(new int(42)) or .reset(new ...)
                    if grandparent.kind() == "call_expression"
                        && let Some(func) = grandparent.child_by_field_name("function") {
                            let func_text = node_text(func, source);
                            if func_text.contains("unique_ptr")
                                || func_text.contains("shared_ptr")
                                || func_text.ends_with("reset")
                            {
                                return true;
                            }
                        }
                    // init_declarator: e.g., std::unique_ptr<int> p(new int(42))
                    if grandparent.kind() == "init_declarator"
                        && let Some(declaration) = grandparent.parent() {
                            let decl_text = node_text(declaration, source);
                            if decl_text.contains("unique_ptr") || decl_text.contains("shared_ptr")
                            {
                                return true;
                            }
                        }
                }
            // Direct parent is init_declarator or declaration
            if parent.kind() == "init_declarator" || parent.kind() == "declaration" {
                let text = node_text(parent, source);
                if text.contains("unique_ptr") || text.contains("shared_ptr") {
                    return true;
                }
            }
            // Check grandparent declaration too
            if let Some(gp) = parent.parent() {
                let text = node_text(gp, source);
                if text.contains("unique_ptr") || text.contains("shared_ptr") {
                    return true;
                }
            }
        }
        false
    }

    fn walk_for_manual_lock(
        node: tree_sitter::Node,
        source: &[u8],
        fn_body_text: &str,
        findings: &mut Vec<AuditFinding>,
        file_path: &str,
        pipeline_name: &str,
    ) {
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function") {
                let func_text = node_text(func, source);
                if func_text.ends_with(".lock") || func_text.ends_with("->lock") {
                    // Check if the function body has RAII lock guards
                    let has_raii_lock = fn_body_text.contains("lock_guard")
                        || fn_body_text.contains("unique_lock")
                        || fn_body_text.contains("scoped_lock");
                    if !has_raii_lock {
                        let start = node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "mutex_without_lock_guard".to_string(),
                            message: "manual `mutex.lock()` without `lock_guard` — exception or early return will deadlock".to_string(),
                            snippet: extract_snippet(source, node, 1),
                        });
                    }
                    return;
                }
            }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_manual_lock(
                child,
                source,
                fn_body_text,
                findings,
                file_path,
                pipeline_name,
            );
        }
    }

    fn walk_for_fopen(
        node: tree_sitter::Node,
        source: &[u8],
        findings: &mut Vec<AuditFinding>,
        file_path: &str,
        pipeline_name: &str,
    ) {
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function") {
                let func_text = node_text(func, source);
                if func_text == "fopen" {
                    // Check if the result is wrapped in a smart pointer
                    let in_smart_ptr = if let Some(parent) = node.parent() {
                        let parent_text = node_text(parent, source);
                        if parent_text.contains("unique_ptr") || parent_text.contains("shared_ptr")
                        {
                            true
                        } else if let Some(gp) = parent.parent() {
                            let gp_text = node_text(gp, source);
                            gp_text.contains("unique_ptr") || gp_text.contains("shared_ptr")
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !in_smart_ptr {
                        let start = node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: pipeline_name.to_string(),
                            pattern: "file_handle_without_raii".to_string(),
                            message: "raw `FILE*` from `fopen()` — use RAII wrapper to ensure `fclose()` on all exit paths".to_string(),
                            snippet: extract_snippet(source, node, 1),
                        });
                    }
                    return;
                }
            }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_for_fopen(child, source, findings, file_path, pipeline_name);
        }
    }
}

impl Pipeline for CppExceptionSafetyPipeline {
    fn name(&self) -> &str {
        "cpp_exception_safety"
    }

    fn description(&self) -> &str {
        "Detects exception safety issues: raw new without RAII, mutex without lock_guard, FILE* without RAII"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Pattern 1: new_before_throw — raw new without smart pointer wrapper
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.new_query, tree.root_node(), source);
            let new_idx = find_capture_index(&self.new_query, "new_expr");

            while let Some(m) = matches.next() {
                let new_cap = m.captures.iter().find(|c| c.index as usize == new_idx);

                if let Some(new_cap) = new_cap {
                    if Self::is_smart_ptr_context(new_cap.node, source) {
                        continue;
                    }

                    let start = new_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "new_before_throw".to_string(),
                        message:
                            "raw `new` without smart pointer wrapper — resource leak if exception is thrown"
                                .to_string(),
                        snippet: extract_snippet(source, new_cap.node, 1),
                    });
                }
            }
        }

        // Pattern 2 & 3: Walk function bodies for mutex_without_lock_guard and file_handle_without_raii
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);
            let fn_body_idx = find_capture_index(&self.fn_query, "fn_body");

            while let Some(m) = matches.next() {
                let fn_body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

                if let Some(fn_body_cap) = fn_body_cap {
                    let fn_body_text = node_text(fn_body_cap.node, source);

                    Self::walk_for_manual_lock(
                        fn_body_cap.node,
                        source,
                        fn_body_text,
                        &mut findings,
                        file_path,
                        self.name(),
                    );

                    Self::walk_for_fopen(
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
        let pipeline = CppExceptionSafetyPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_raw_new() {
        let src = "void f() { int* p = new int(42); delete p; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "new_before_throw");
    }

    #[test]
    fn ignores_smart_ptr_new() {
        let src = "void f() { auto p = std::make_unique<int>(42); }";
        let findings = parse_and_check(src);
        // make_unique does not involve raw new, so no findings
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_manual_lock() {
        let src = "void f(std::mutex& m) { m.lock(); m.unlock(); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutex_without_lock_guard");
    }

    #[test]
    fn ignores_lock_guard() {
        let src = "void f(std::mutex& m) { std::lock_guard<std::mutex> g(m); }";
        let findings = parse_and_check(src);
        let lock_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "mutex_without_lock_guard")
            .collect();
        assert!(lock_findings.is_empty());
    }

    #[test]
    fn detects_fopen_without_raii() {
        let src = r#"void f() { FILE* fp = fopen("test.txt", "r"); fclose(fp); }"#;
        let findings = parse_and_check(src);
        let fopen_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "file_handle_without_raii")
            .collect();
        assert_eq!(fopen_findings.len(), 1);
    }

    #[test]
    fn skips_smart_ptr_constructor_with_new() {
        let src = "void f() { std::unique_ptr<int> p(new int(42)); }";
        let findings = parse_and_check(src);
        let new_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "new_before_throw")
            .collect();
        assert!(new_findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = "void f() { int* p = new int; }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "cpp_exception_safety");
    }
}
