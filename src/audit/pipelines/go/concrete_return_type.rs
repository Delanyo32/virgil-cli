use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_generated_go_file, is_nolint_suppressed};
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Factory prefixes that are expected to return concrete types.
const FACTORY_PREFIXES: &[&str] = &["New", "Create", "Build", "Make", "Open"];

pub struct ConcreteReturnTypePipeline {
    fn_query: Arc<Query>,
    fn_tuple_query: Arc<Query>,
    method_query: Arc<Query>,
    method_tuple_query: Arc<Query>,
}

impl ConcreteReturnTypePipeline {
    pub fn new() -> Result<Self> {
        let ts_lang = Language::Go.tree_sitter_language();

        // Query 1: function_declaration with direct pointer return
        let fn_query_str = r#"
(function_declaration
  name: (identifier) @fn_name
  result: (pointer_type
    (type_identifier) @return_type)) @fn_decl
"#;
        let fn_query = Query::new(&ts_lang, fn_query_str)
            .with_context(|| "failed to compile concrete return type query for Go")?;

        // Query 2: function_declaration with tuple return containing pointer type
        let fn_tuple_query_str = r#"
(function_declaration
  name: (identifier) @fn_name
  result: (parameter_list
    (parameter_declaration
      type: (pointer_type
        (type_identifier) @return_type)))) @fn_decl
"#;
        let fn_tuple_query = Query::new(&ts_lang, fn_tuple_query_str)
            .with_context(|| "failed to compile concrete return type tuple query for Go")?;

        // Query 3: method_declaration with direct pointer return
        let method_query_str = r#"
(method_declaration
  name: (field_identifier) @fn_name
  result: (pointer_type
    (type_identifier) @return_type)) @fn_decl
"#;
        let method_query = Query::new(&ts_lang, method_query_str)
            .with_context(|| "failed to compile concrete return type method query for Go")?;

        // Query 4: method_declaration with tuple return containing pointer type
        let method_tuple_query_str = r#"
(method_declaration
  name: (field_identifier) @fn_name
  result: (parameter_list
    (parameter_declaration
      type: (pointer_type
        (type_identifier) @return_type)))) @fn_decl
"#;
        let method_tuple_query = Query::new(&ts_lang, method_tuple_query_str)
            .with_context(|| "failed to compile concrete return type method tuple query for Go")?;

        Ok(Self {
            fn_query: Arc::new(fn_query),
            fn_tuple_query: Arc::new(fn_tuple_query),
            method_query: Arc::new(method_query),
            method_tuple_query: Arc::new(method_tuple_query),
        })
    }

    fn check_query(
        &self,
        query: &Query,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        let name_idx = find_capture_index(query, "fn_name");
        let return_idx = find_capture_index(query, "return_type");
        let decl_idx = find_capture_index(query, "fn_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == name_idx)
                .map(|c| c.node);
            let return_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == return_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(return_node), Some(decl_node)) =
                (name_node, return_node, decl_node)
            {
                let fn_name = node_text(name_node, source);

                // Only flag exported functions (starts with uppercase)
                let first_char = fn_name.chars().next().unwrap_or('a');
                if !first_char.is_uppercase() {
                    continue;
                }

                // Skip factory functions (Go constructor conventions)
                if FACTORY_PREFIXES.iter().any(|p| fn_name.starts_with(p)) {
                    continue;
                }

                // Skip nolint-suppressed declarations
                if is_nolint_suppressed(source, decl_node, self.name()) {
                    continue;
                }

                let return_type = node_text(return_node, source);
                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "exported_concrete_pointer_return".to_string(),
                    message: format!(
                        "`{fn_name}` returns `*{return_type}` — consider returning an interface for flexibility"
                    ),
                    snippet: extract_snippet(source, decl_node, 1),
                });
            }
        }

        findings
    }
}

impl GraphPipeline for ConcreteReturnTypePipeline {
    fn name(&self) -> &str {
        "concrete_return_type"
    }

    fn description(&self) -> &str {
        "Detects exported functions returning *ConcreteType instead of interface"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        // Skip generated Go files
        if is_generated_go_file(file_path, source) {
            return vec![];
        }

        // Internal packages don't need interface returns
        if file_path.contains("/internal/") || file_path.starts_with("internal/") {
            return vec![];
        }

        let mut findings = Vec::new();

        // Run all four queries and merge findings
        findings.extend(self.check_query(&self.fn_query, tree, source, file_path));
        findings.extend(self.check_query(&self.fn_tuple_query, tree, source, file_path));
        findings.extend(self.check_query(&self.method_query, tree, source, file_path));
        findings.extend(self.check_query(&self.method_tuple_query, tree, source, file_path));

        // Deduplicate by (file_path, line, column) in case multiple queries match the same decl
        findings.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));
        findings.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.file_path == b.file_path);

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
        let pipeline = ConcreteReturnTypePipeline::new().unwrap();
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
    fn detects_exported_concrete_return() {
        let src =
            "package main\ntype RedisCache struct{}\nfunc GetCache() *RedisCache { return nil }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "exported_concrete_pointer_return");
        assert!(findings[0].message.contains("GetCache"));
    }

    #[test]
    fn skips_new_constructor() {
        let src =
            "package main\ntype RedisCache struct{}\nfunc NewCache() *RedisCache { return nil }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_interface_return() {
        let src = "package main\ntype Cache interface{ Get(string) string }\ntype RedisCache struct{}\nfunc NewCache() Cache { return &RedisCache{} }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_unexported_function() {
        let src =
            "package main\ntype RedisCache struct{}\nfunc newCache() *RedisCache { return nil }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression_skips_finding() {
        let src = "package main\nfunc GetCache() *RedisCache { return nil } // NOLINT(concrete_return_type)\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc GetCache() *RedisCache { return nil }\n";
        let findings = parse_and_check_file(src, "cache.pb.go");
        assert!(findings.is_empty());
    }

    #[test]
    fn internal_package_skipped() {
        let src = "package cache\nfunc GetCache() *RedisCache { return nil }\n";
        let findings = parse_and_check_file(src, "internal/cache/redis.go");
        assert!(findings.is_empty());
    }

    #[test]
    fn create_factory_not_flagged() {
        let src = "package main\nfunc CreateCache() *RedisCache { return nil }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn build_factory_not_flagged() {
        let src = "package main\nfunc BuildCache() *RedisCache { return nil }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn tuple_return_detected() {
        let src = "package main\nfunc GetCache() (*RedisCache, error) { return nil, nil }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "exported_concrete_pointer_return");
    }
}
