use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_call_expression_query, compile_delete_expression_query, compile_new_expression_query,
    extract_snippet, find_capture_index, node_text,
};

const C_ALLOC_FUNCTIONS: &[&str] = &["malloc", "calloc", "realloc", "free"];

pub struct RawMemoryManagementPipeline {
    new_query: Arc<Query>,
    delete_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl RawMemoryManagementPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            new_query: compile_new_expression_query()?,
            delete_query: compile_delete_expression_query()?,
            call_query: compile_call_expression_query()?,
        })
    }

    fn is_smart_ptr_init(node: tree_sitter::Node, source: &[u8]) -> bool {
        if let Some(parent) = node.parent()
            && parent.kind() == "argument_list"
            && let Some(grandparent) = parent.parent()
        {
            if grandparent.kind() == "call_expression"
                && let Some(func) = grandparent.child_by_field_name("function")
            {
                let func_text = node_text(func, source);
                if func_text.contains("unique_ptr")
                    || func_text.contains("shared_ptr")
                    || func_text.ends_with("reset")
                {
                    return true;
                }
            }
            if grandparent.kind() == "init_declarator"
                && let Some(declaration) = grandparent.parent()
            {
                let decl_text = node_text(declaration, source);
                if decl_text.contains("unique_ptr") || decl_text.contains("shared_ptr") {
                    return true;
                }
            }
        }
        false
    }

    fn is_inside_destructor(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_definition" {
                if let Some(declarator) = parent.child_by_field_name("declarator") {
                    let decl_text = node_text(declarator, source);
                    if decl_text.contains('~') {
                        return true;
                    }
                    // Also check for destructor_name node kind
                    let mut cursor = declarator.walk();
                    for child in declarator.children(&mut cursor) {
                        if child.kind() == "destructor_name" {
                            return true;
                        }
                    }
                }
                return false;
            }
            current = parent.parent();
        }
        false
    }
}

impl GraphPipeline for RawMemoryManagementPipeline {
    fn name(&self) -> &str {
        "raw_memory_management"
    }

    fn description(&self) -> &str {
        "Detects raw new/delete and C-style malloc/free — prefer smart pointers and std::make_unique/std::make_shared"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();

        // Detect raw `new`
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.new_query, tree.root_node(), source);
            let new_idx = find_capture_index(&self.new_query, "new_expr");

            while let Some(m) = matches.next() {
                let new_cap = m.captures.iter().find(|c| c.index as usize == new_idx);

                if let Some(new_cap) = new_cap {
                    if Self::is_smart_ptr_init(new_cap.node, source) {
                        continue;
                    }

                    if is_nolint_suppressed(source, new_cap.node, self.name()) {
                        continue;
                    }

                    let text = node_text(new_cap.node, source);
                    let pattern = if text.contains('[') {
                        "raw_array_allocation"
                    } else {
                        "raw_new_delete"
                    };

                    let start = new_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: "raw `new` — use `std::make_unique` or `std::make_shared` instead"
                            .to_string(),
                        snippet: extract_snippet(source, new_cap.node, 1),
                    });
                }
            }
        }

        // Detect raw `delete`
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.delete_query, tree.root_node(), source);
            let delete_idx = find_capture_index(&self.delete_query, "delete_expr");

            while let Some(m) = matches.next() {
                let del_cap = m.captures.iter().find(|c| c.index as usize == delete_idx);

                if let Some(del_cap) = del_cap {
                    if is_nolint_suppressed(source, del_cap.node, self.name()) {
                        continue;
                    }

                    // delete in destructor is expected RAII cleanup — lower severity
                    let severity = if Self::is_inside_destructor(del_cap.node, source) {
                        "info"
                    } else {
                        "warning"
                    };

                    let start = del_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "raw_new_delete".to_string(),
                        message: "raw `delete` — smart pointers handle deallocation automatically"
                            .to_string(),
                        snippet: extract_snippet(source, del_cap.node, 1),
                    });
                }
            }
        }

        // Detect C-style allocation functions (malloc, calloc, realloc, free)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);
            let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
            let call_idx = find_capture_index(&self.call_query, "call");

            while let Some(m) = matches.next() {
                let fn_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_name_idx);
                let call_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx);

                if let (Some(fn_cap), Some(call_cap)) = (fn_cap, call_cap) {
                    let fn_text = node_text(fn_cap.node, source);

                    if !C_ALLOC_FUNCTIONS.contains(&fn_text) {
                        continue;
                    }

                    if is_nolint_suppressed(source, call_cap.node, self.name()) {
                        continue;
                    }

                    let start = call_cap.node.start_position();
                    let message = if fn_text == "free" {
                        "C-style `free()` in C++ — use smart pointers for automatic deallocation"
                            .to_string()
                    } else {
                        format!(
                            "C-style `{fn_text}()` in C++ — use `std::make_unique` or container types instead"
                        )
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "c_style_allocation".to_string(),
                        message,
                        snippet: extract_snippet(source, call_cap.node, 1),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RawMemoryManagementPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.cpp",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_raw_new() {
        let src = "void f() { int* p = new int(42); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_new_delete");
        assert!(findings[0].message.contains("new"));
    }

    #[test]
    fn detects_raw_delete() {
        let src = "void f(int* p) { delete p; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_new_delete");
        assert!(findings[0].message.contains("delete"));
    }

    #[test]
    fn detects_array_new() {
        let src = "void f() { int* arr = new int[100]; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_array_allocation");
    }

    #[test]
    fn no_finding_for_make_unique() {
        let src = "void f() { auto p = std::make_unique<int>(42); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_smart_ptr_constructor() {
        let src = "void f() { std::unique_ptr<int> p(new int(42)); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_both_new_and_delete() {
        let src = r#"
void f() {
    int* p = new int(10);
    delete p;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn metadata_correct() {
        let src = "void f() { int* p = new int; }";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "raw_memory_management");
    }

    #[test]
    fn detects_malloc_in_cpp() {
        let src = "void f() { void* p = malloc(100); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "c_style_allocation");
        assert!(findings[0].message.contains("malloc"));
    }

    #[test]
    fn detects_free_in_cpp() {
        let src = "void f(void* p) { free(p); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "c_style_allocation");
    }

    #[test]
    fn delete_in_destructor_info() {
        let src = r#"
class Foo {
    int* data;
    ~Foo() { delete data; }
};
"#;
        let findings = parse_and_check(src);
        let del_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "raw_new_delete")
            .collect();
        assert_eq!(del_findings.len(), 1);
        assert_eq!(del_findings[0].severity, "info");
    }

    #[test]
    fn nolint_suppression() {
        let src = "void f() { int* p = new int; // NOLINT }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
