use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_function_definition_query, compile_parameter_declaration_query, extract_snippet,
    find_capture_index, is_pointer_declarator, node_text,
};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

/// Common callback context parameter names that are idiomatic void* usage.
const CALLBACK_PARAM_NAMES: &[&str] = &[
    "ctx", "context", "data", "arg", "userdata", "user_data", "opaque", "priv", "cookie",
    "closure", "state",
];

pub struct VoidPointerAbusePipeline {
    param_query: Arc<Query>,
    fn_def_query: Arc<Query>,
}

impl VoidPointerAbusePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_parameter_declaration_query()?,
            fn_def_query: compile_function_definition_query()?,
        })
    }

    /// Check if a function definition has a function pointer parameter (indicating callback pattern).
    fn has_function_pointer_param(fn_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk the function's declarator to find the parameter_list,
        // then check if any parameter contains a function_declarator
        let declarator = match fn_node.child_by_field_name("declarator") {
            Some(d) => d,
            None => return false,
        };
        Self::find_function_pointer_in_params(declarator, source)
    }

    fn find_function_pointer_in_params(node: tree_sitter::Node, _source: &[u8]) -> bool {
        if node.kind() == "function_declarator" {
            if let Some(params) = node.child_by_field_name("parameters") {
                let mut cursor = params.walk();
                for param in params.named_children(&mut cursor) {
                    if Self::subtree_has_kind(param, "function_declarator") {
                        return true;
                    }
                }
            }
            return false;
        }
        // Recurse into pointer_declarator etc. to find the function_declarator
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::find_function_pointer_in_params(inner, _source);
        }
        false
    }

    fn subtree_has_kind(node: tree_sitter::Node, kind: &str) -> bool {
        if node.kind() == kind {
            return true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::subtree_has_kind(child, kind) {
                return true;
            }
        }
        false
    }

    /// Check if the parameter declaration has a const qualifier.
    fn is_const_void_pointer(param_decl: tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = param_decl.walk();
        for child in param_decl.children(&mut cursor) {
            if child.kind() == "type_qualifier" {
                let text = child.utf8_text(source).unwrap_or("");
                if text == "const" {
                    return true;
                }
            }
        }
        false
    }

    /// Extract the parameter name from a parameter_declaration.
    fn extract_param_name<'a>(declarator_node: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
        // Walk down through pointer_declarator to find the identifier
        let mut node = declarator_node;
        loop {
            if node.kind() == "identifier" {
                return node.utf8_text(source).unwrap_or("");
            }
            if let Some(inner) = node.child_by_field_name("declarator") {
                node = inner;
            } else {
                return node.utf8_text(source).unwrap_or("");
            }
        }
    }
}

impl GraphPipeline for VoidPointerAbusePipeline {
    fn name(&self) -> &str {
        "void_pointer_abuse"
    }

    fn description(&self) -> &str {
        "Detects void* parameters and return types that bypass type safety"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();

        // Check void* parameters
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.param_query, tree.root_node(), source);

            let param_type_idx = find_capture_index(&self.param_query, "param_type");
            let param_decl_idx = find_capture_index(&self.param_query, "param_decl");
            let param_declarator_idx = find_capture_index(&self.param_query, "param_declarator");

            while let Some(m) = matches.next() {
                let type_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_type_idx);
                let decl_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_decl_idx);
                let declarator_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_declarator_idx);

                if let (Some(type_cap), Some(decl_cap)) = (type_cap, decl_cap) {
                    let type_text = node_text(type_cap.node, source).trim();

                    if type_text != "void" {
                        continue;
                    }

                    let has_pointer = declarator_cap
                        .map(|c| is_pointer_declarator(c.node))
                        .unwrap_or(false);

                    if !has_pointer {
                        continue;
                    }

                    // Skip const void* (read-only generic data, e.g. qsort comparator)
                    if Self::is_const_void_pointer(decl_cap.node, source) {
                        continue;
                    }

                    // Extract param name for callback context detection
                    if let Some(decl_node) = declarator_cap {
                        let param_name = Self::extract_param_name(decl_node.node, source);
                        // Skip callback context patterns
                        if CALLBACK_PARAM_NAMES.contains(&param_name)
                            && let Some(fn_node) = Self::find_enclosing_function(decl_cap.node)
                            && Self::has_function_pointer_param(fn_node, source)
                        {
                            continue;
                        }
                    }

                    if is_nolint_suppressed(source, decl_cap.node, self.name()) {
                        continue;
                    }

                    let start = decl_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "void_pointer_parameter".to_string(),
                        message: "void* parameter bypasses type safety — consider using a concrete type or typed callback".to_string(),
                        snippet: extract_snippet(source, decl_cap.node, 1),
                    });
                }
            }
        }

        // Check void* return types
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

            let declarator_idx = find_capture_index(&self.fn_def_query, "declarator");
            let fn_def_idx = find_capture_index(&self.fn_def_query, "fn_def");

            while let Some(m) = matches.next() {
                let decl_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == declarator_idx);
                let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_def_idx);

                if let (Some(decl_cap), Some(fn_cap)) = (decl_cap, fn_cap) {
                    let fn_node = fn_cap.node;

                    let type_node = fn_node.child_by_field_name("type");
                    let type_text = type_node
                        .map(|n| node_text(n, source).trim().to_string())
                        .unwrap_or_default();

                    if type_text != "void" {
                        continue;
                    }

                    if decl_cap.node.kind() != "pointer_declarator" {
                        continue;
                    }

                    if is_nolint_suppressed(source, fn_cap.node, self.name()) {
                        continue;
                    }

                    let start = fn_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "void_pointer_return".to_string(),
                        message:
                            "function returns void* — callers must cast, which bypasses type safety"
                                .to_string(),
                        snippet: extract_snippet(source, fn_cap.node, 1),
                    });
                }
            }
        }

        findings
    }
}

impl VoidPointerAbusePipeline {
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = VoidPointerAbusePipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.c",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_void_pointer_param() {
        let src = "void process(void *data) {}";
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "void_pointer_parameter")
        );
    }

    #[test]
    fn detects_void_pointer_return() {
        let src = "void *create() { return 0; }";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "void_pointer_return"));
    }

    #[test]
    fn skips_int_pointer_param() {
        let src = "void process(int *data) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_void_no_pointer() {
        let src = "void func(void) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_const_void_pointer() {
        let src = "int compare(const void *a, const void *b) { return 0; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_callback_context_with_fn_pointer() {
        let src = "void register_handler(void (*callback)(int), void *ctx) {}";
        let findings = parse_and_check(src);
        // The void *ctx should be skipped because of callback pattern
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "void_pointer_parameter")
        );
    }

    #[test]
    fn distinct_pattern_for_return_vs_param() {
        let src = r#"
void *create() { return 0; }
void process(void *data) {}
"#;
        let findings = parse_and_check(src);
        let has_return = findings.iter().any(|f| f.pattern == "void_pointer_return");
        let has_param = findings.iter().any(|f| f.pattern == "void_pointer_parameter");
        assert!(has_return);
        assert!(has_param);
    }

    #[test]
    fn nolint_suppresses() {
        let src = "void process(void *data) {} // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
