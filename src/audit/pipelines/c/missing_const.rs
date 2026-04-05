use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_parameter_declaration_query, extract_snippet, find_capture_index, has_type_qualifier,
    is_pointer_declarator, node_text,
};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

pub struct MissingConstPipeline {
    param_query: Arc<Query>,
}

impl MissingConstPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_parameter_declaration_query()?,
        })
    }

    /// Count the depth of nested pointer_declarator nodes.
    fn pointer_depth(node: tree_sitter::Node) -> usize {
        if node.kind() == "pointer_declarator" {
            if let Some(inner) = node.child_by_field_name("declarator") {
                return 1 + Self::pointer_depth(inner);
            }
            return 1;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::pointer_depth(inner);
        }
        0
    }

    /// Extract the parameter name from a declarator node.
    fn extract_param_name<'a>(node: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
        let mut current = node;
        loop {
            if current.kind() == "identifier" {
                return current.utf8_text(source).unwrap_or("");
            }
            if let Some(inner) = current.child_by_field_name("declarator") {
                current = inner;
            } else {
                return current.utf8_text(source).unwrap_or("");
            }
        }
    }

    /// Check if the function body writes through a pointer parameter.
    /// Looks for: *param = ..., param[i] = ..., param->field = ...
    fn body_writes_through_param(fn_node: tree_sitter::Node, param_name: &str, source: &[u8]) -> bool {
        if let Some(body) = fn_node.child_by_field_name("body") {
            return Self::subtree_writes_through(body, param_name, source);
        }
        false
    }

    fn subtree_writes_through(node: tree_sitter::Node, param_name: &str, source: &[u8]) -> bool {
        // Check assignment expressions where the LHS dereferences the parameter
        if node.kind() == "assignment_expression"
            && let Some(left) = node.child_by_field_name("left")
            && Self::lhs_dereferences_param(left, param_name, source)
        {
            return true;
        }

        // Check update expressions (++, --) on dereferenced param
        if node.kind() == "update_expression" {
            let text = node.utf8_text(source).unwrap_or("");
            if text.contains(param_name) && (text.contains('*') || text.contains('[')) {
                return true;
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::subtree_writes_through(child, param_name, source) {
                return true;
            }
        }
        false
    }

    /// Check if a LHS expression dereferences the given parameter.
    fn lhs_dereferences_param(lhs: tree_sitter::Node, param_name: &str, source: &[u8]) -> bool {
        match lhs.kind() {
            // *param = ...
            "pointer_expression" => {
                let text = lhs.utf8_text(source).unwrap_or("");
                text.contains(param_name)
            }
            // param[i] = ...
            "subscript_expression" => {
                if let Some(arg) = lhs.child_by_field_name("argument") {
                    let text = node_text(arg, source);
                    text == param_name
                } else {
                    false
                }
            }
            // param->field = ...
            "field_expression" => {
                if let Some(arg) = lhs.child_by_field_name("argument") {
                    let text = node_text(arg, source);
                    text == param_name
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

impl GraphPipeline for MissingConstPipeline {
    fn name(&self) -> &str {
        "missing_const"
    }

    fn description(&self) -> &str {
        "Detects non-const pointer parameters that could be const for safety and clarity"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
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

                // Skip void* (caught by void_pointer_abuse pipeline)
                if type_text == "void" {
                    continue;
                }

                // Must be a pointer parameter
                let has_pointer = declarator_cap
                    .map(|c| is_pointer_declarator(c.node))
                    .unwrap_or(false);

                if !has_pointer {
                    continue;
                }

                // Skip if already has const qualifier
                if has_type_qualifier(decl_cap.node, source, "const") {
                    continue;
                }

                // Skip double+ pointers (likely output parameters)
                if let Some(decl_node) = declarator_cap {
                    if Self::pointer_depth(decl_node.node) >= 2 {
                        continue;
                    }

                    // Extract param name
                    let param_name = Self::extract_param_name(decl_node.node, source);

                    // Skip output parameter naming conventions
                    if param_name.starts_with("out_") || param_name.starts_with("out")
                        || param_name == "result" || param_name == "output"
                    {
                        continue;
                    }

                    // Check if the function body writes through this pointer
                    if let Some(fn_node) = Self::find_enclosing_function(decl_cap.node)
                        && Self::body_writes_through_param(fn_node, param_name, source)
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
                    pattern: "missing_const_param".to_string(),
                    message: "non-const pointer parameter — add `const` if the function does not modify the data".to_string(),
                    snippet: extract_snippet(source, decl_cap.node, 1),
                });
            }
        }

        findings
    }
}

impl MissingConstPipeline {
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
        let pipeline = MissingConstPipeline::new().unwrap();
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
    fn detects_read_only_pointer_param() {
        let src = r#"
int sum(int *arr, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += arr[i];
    return s;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_const_param");
    }

    #[test]
    fn skips_const_pointer() {
        let src = "void process(const char *data) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_pointer() {
        let src = "void process(int n) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_void_pointer() {
        let src = "void process(void *data) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_output_parameter() {
        // Function writes through the pointer — should NOT flag
        let src = "void init(int *out) { *out = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_array_write() {
        let src = "void fill(int *arr, int n) { arr[0] = 1; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_field_write() {
        let src = "void reset(struct Foo *f) { f->count = 0; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_double_pointer() {
        let src = "void alloc(int **out) { *out = malloc(10); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_out_prefix() {
        let src = "void get_name(char *out_name) {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses() {
        let src = "void process(char *data) {} // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
