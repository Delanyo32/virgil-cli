use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

pub struct ConcreteReturnTypePipeline {
    fn_query: Arc<Query>,
}

impl ConcreteReturnTypePipeline {
    pub fn new() -> Result<Self> {
        let ts_lang = Language::Go.tree_sitter_language();
        let query_str = r#"
(function_declaration
  name: (identifier) @fn_name
  result: (pointer_type
    (type_identifier) @return_type)) @fn_decl
"#;
        let query = Query::new(&ts_lang, query_str)
            .with_context(|| "failed to compile concrete return type query for Go")?;
        Ok(Self {
            fn_query: Arc::new(query),
        })
    }
}

impl Pipeline for ConcreteReturnTypePipeline {
    fn name(&self) -> &str {
        "concrete_return_type"
    }

    fn description(&self) -> &str {
        "Detects exported functions returning *ConcreteType instead of interface"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.fn_query, "fn_name");
        let return_idx = find_capture_index(&self.fn_query, "return_type");
        let decl_idx = find_capture_index(&self.fn_query, "fn_decl");

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == name_idx).map(|c| c.node);
            let return_node = m.captures.iter().find(|c| c.index as usize == return_idx).map(|c| c.node);
            let decl_node = m.captures.iter().find(|c| c.index as usize == decl_idx).map(|c| c.node);

            if let (Some(name_node), Some(return_node), Some(decl_node)) = (name_node, return_node, decl_node) {
                let fn_name = node_text(name_node, source);

                // Only flag exported functions (starts with uppercase)
                let first_char = fn_name.chars().next().unwrap_or('a');
                if !first_char.is_uppercase() {
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
        let pipeline = ConcreteReturnTypePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_exported_concrete_return() {
        let src = "package main\ntype RedisCache struct{}\nfunc NewCache() *RedisCache { return nil }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "exported_concrete_pointer_return");
        assert!(findings[0].message.contains("NewCache"));
    }

    #[test]
    fn clean_interface_return() {
        let src = "package main\ntype Cache interface{ Get(string) string }\ntype RedisCache struct{}\nfunc NewCache() Cache { return &RedisCache{} }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_unexported_function() {
        let src = "package main\ntype RedisCache struct{}\nfunc newCache() *RedisCache { return nil }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
