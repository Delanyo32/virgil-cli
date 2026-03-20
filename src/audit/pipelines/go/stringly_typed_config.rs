use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_field_decl_query, compile_param_decl_query, extract_snippet, find_capture_index,
    node_text,
};

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

    fn is_map_string_string(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() != "map_type" {
            return false;
        }
        let key = node.child_by_field_name("key");
        let value = node.child_by_field_name("value");
        match (key, value) {
            (Some(k), Some(v)) => {
                node_text(k, source) == "string" && node_text(v, source) == "string"
            }
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_with_query(
        &self,
        tree: &Tree,
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
                && Self::is_map_string_string(type_node, source)
            {
                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message: "`map[string]string` — consider a typed struct for configuration"
                        .to_string(),
                    snippet: extract_snippet(source, decl_node, 1),
                });
            }
        }

        findings
    }
}

impl Pipeline for StringlyTypedConfigPipeline {
    fn name(&self) -> &str {
        "stringly_typed_config"
    }

    fn description(&self) -> &str {
        "Detects `map[string]string` in params/fields — stringly-typed configuration"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_with_query(
            tree,
            source,
            &self.param_query,
            "param_type",
            "param",
            file_path,
            "string_map_param",
        ));
        findings.extend(self.check_with_query(
            tree,
            source,
            &self.field_query,
            "field_type",
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
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = StringlyTypedConfigPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
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
}
