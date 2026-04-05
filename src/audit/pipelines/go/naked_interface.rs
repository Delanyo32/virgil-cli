use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_generated_go_file, is_nolint_suppressed, is_test_file};

use super::primitives::{
    compile_field_decl_query, compile_param_decl_query, extract_snippet, find_capture_index,
    node_text,
};

pub struct NakedInterfacePipeline {
    param_query: Arc<Query>,
    field_query: Arc<Query>,
}

impl NakedInterfacePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_param_decl_query()?,
            field_query: compile_field_decl_query()?,
        })
    }

    fn is_naked_interface_leaf(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Check for `interface{}` — an interface_type with no method specs
        if node.kind() == "interface_type" {
            return node.named_child_count() == 0;
        }
        // Check for `any` type identifier
        if node.kind() == "type_identifier" && node_text(node, source) == "any" {
            return true;
        }
        false
    }

    /// Check if a node is or contains a naked interface.
    /// Recursively checks slice_type elements and map_type values.
    fn is_or_contains_naked_interface(node: tree_sitter::Node, source: &[u8]) -> bool {
        if Self::is_naked_interface_leaf(node, source) {
            return true;
        }
        // []interface{} or []any
        if node.kind() == "slice_type" {
            if let Some(elem) = node.child_by_field_name("element") {
                return Self::is_or_contains_naked_interface(elem, source);
            }
        }
        // map[K]interface{} or map[K]any
        if node.kind() == "map_type" {
            if let Some(val) = node.child_by_field_name("value") {
                return Self::is_or_contains_naked_interface(val, source);
            }
        }
        false
    }

    /// Check if a node is inside an exported function (name starts with uppercase).
    fn is_in_exported_function(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node;
        loop {
            let kind = current.kind();
            if kind == "function_declaration" || kind == "method_declaration" {
                if let Some(name_node) = current.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    if let Some(first) = name.chars().next() {
                        return first.is_uppercase();
                    }
                }
                return false;
            }
            match current.parent() {
                Some(p) => current = p,
                None => return false,
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_with_query(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        query: &Query,
        type_capture: &str,
        decl_capture: &str,
        file_path: &str,
        pattern: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        let type_idx = find_capture_index(query, type_capture);
        let decl_idx = find_capture_index(query, decl_capture);

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == decl_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(decl_node)) = (type_node, decl_node)
                && Self::is_or_contains_naked_interface(type_node, source)
            {
                if is_nolint_suppressed(source, decl_node, self.name()) {
                    continue;
                }
                let severity = if Self::is_in_exported_function(decl_node, source) {
                    "warning"
                } else {
                    "info"
                };
                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message: "empty interface (`interface{}` / `any`) loses type safety — consider a concrete interface".to_string(),
                    snippet: extract_snippet(source, decl_node, 1),
                });
            }
        }

        findings
    }
}

impl GraphPipeline for NakedInterfacePipeline {
    fn name(&self) -> &str {
        "naked_interface"
    }

    fn description(&self) -> &str {
        "Detects use of `interface{}` or `any` as parameter or field types"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_generated_go_file(file_path, source) {
            return vec![];
        }
        if is_test_file(file_path) {
            return vec![];
        }

        let mut findings = Vec::new();
        findings.extend(self.check_with_query(
            tree,
            source,
            &self.param_query,
            "param_type",
            "param",
            file_path,
            "empty_interface_param",
        ));
        findings.extend(self.check_with_query(
            tree,
            source,
            &self.field_query,
            "field_type",
            "field",
            file_path,
            "empty_interface_field",
        ));
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_file(source, "test.go")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NakedInterfacePipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_empty_interface_param() {
        let src = "package main\nfunc Process(items interface{}) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_interface_param");
    }

    #[test]
    fn detects_any_param() {
        let src = "package main\nfunc Process(items any) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_interface_param");
    }

    #[test]
    fn clean_concrete_interface() {
        let src = "package main\ntype Stringer interface { String() string }\nfunc Process(items Stringer) {}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_empty_interface_field() {
        let src = "package main\ntype Config struct {\n\tValue interface{}\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_slice_of_interface() {
        let src = "package main\nfunc Process(items []interface{}) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_map_string_interface() {
        let src = "package main\nfunc Process(data map[string]interface{}) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn nolint_suppression_skips_finding() {
        let src = "package main\nfunc Process(v any) {} // NOLINT(naked_interface)\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc Process(v any) {}\n";
        let findings = parse_and_check_file(src, "types.pb.go");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_file_skipped() {
        let src = "package main\nfunc Process(v any) {}\n";
        let findings = parse_and_check_file(src, "handler_test.go");
        assert!(findings.is_empty());
    }
}
