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

/// Names that are legitimate uses of map[string]string (case-insensitive).
const LEGITIMATE_NAMES: &[&str] = &[
    "headers",
    "labels",
    "tags",
    "annotations",
    "metadata",
    "env",
    "envvars",
    "query",
    "queryparams",
];

pub struct StringlyTypedConfigPipeline {
    param_query: Arc<Query>,
    field_query: Arc<Query>,
}

impl StringlyTypedConfigPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_param_decl_query()?,
            field_query: compile_field_decl_query()?,
        })
    }

    fn is_stringly_typed_map(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() != "map_type" {
            return false;
        }
        let key = node.child_by_field_name("key");
        let value = node.child_by_field_name("value");
        match (key, value) {
            (Some(k), Some(v)) => {
                if node_text(k, source) != "string" {
                    return false;
                }
                let v_kind = v.kind();
                let v_text = node_text(v, source);
                // map[string]string
                if v_kind == "type_identifier" && v_text == "string" {
                    return true;
                }
                // map[string]any
                if v_kind == "type_identifier" && v_text == "any" {
                    return true;
                }
                // map[string]interface{}
                if v_kind == "interface_type" && v.named_child_count() == 0 {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_with_query(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        query: &Query,
        type_capture: &str,
        name_capture: &str,
        decl_capture: &str,
        file_path: &str,
        pattern: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        let type_idx = find_capture_index(query, type_capture);
        let decl_idx = find_capture_index(query, decl_capture);
        let name_idx = find_capture_index(query, name_capture);

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
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == name_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(decl_node)) = (type_node, decl_node)
                && Self::is_stringly_typed_map(type_node, source)
            {
                // Skip legitimate names
                if let Some(nn) = name_node {
                    let name_lower = node_text(nn, source).to_lowercase();
                    if LEGITIMATE_NAMES.contains(&name_lower.as_str()) {
                        continue;
                    }
                }

                // Skip nolint-suppressed findings
                if is_nolint_suppressed(source, decl_node, self.name()) {
                    continue;
                }

                let start = decl_node.start_position();
                let type_text = node_text(type_node, source);
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message: format!(
                        "`{}` — consider a typed struct for configuration",
                        type_text
                    ),
                    snippet: extract_snippet(source, decl_node, 1),
                });
            }
        }

        findings
    }
}

impl GraphPipeline for StringlyTypedConfigPipeline {
    fn name(&self) -> &str {
        "stringly_typed_config"
    }

    fn description(&self) -> &str {
        "Detects `map[string]string` in params/fields — stringly-typed configuration"
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
            "param_name",
            "param",
            file_path,
            "string_map_param",
        ));
        findings.extend(self.check_with_query(
            tree,
            source,
            &self.field_query,
            "field_type",
            "field_name",
            "field",
            file_path,
            "string_map_field",
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
        let pipeline = StringlyTypedConfigPipeline::new().unwrap();
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
    fn detects_param_map_string_string() {
        let src = "package main\nfunc NewService(cfg map[string]string) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "string_map_param");
    }

    #[test]
    fn clean_typed_config() {
        let src = "package main\ntype Config struct { Port int }\nfunc NewService(cfg Config) {}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_map_string_int() {
        let src = "package main\nfunc Process(data map[string]int) {}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_field_map_string_string() {
        let src = "package main\ntype Config struct {\n\tOpts map[string]string\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_map_string_interface() {
        let src = "package main\nfunc New(cfg map[string]interface{}) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_map_string_any() {
        let src = "package main\nfunc New(cfg map[string]any) {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn labels_field_not_flagged() {
        let src = "package main\ntype Pod struct {\n\tLabels map[string]string\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn headers_param_not_flagged() {
        let src = "package main\nfunc SetHeaders(headers map[string]string) {}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression_skips_finding() {
        let src =
            "package main\nfunc New(cfg map[string]string) {} // NOLINT(stringly_typed_config)\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc New(cfg map[string]string) {}\n";
        let findings = parse_and_check_file(src, "config.pb.go");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_file_skipped() {
        let src = "package main\nfunc New(cfg map[string]string) {}\n";
        let findings = parse_and_check_file(src, "config_test.go");
        assert!(findings.is_empty());
    }
}
