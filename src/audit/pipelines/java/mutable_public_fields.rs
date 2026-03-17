use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::java_primitives::{
    compile_field_decl_query, extract_snippet, find_capture_index, has_modifier, node_text,
};

pub struct MutablePublicFieldsPipeline {
    field_query: Arc<Query>,
}

impl MutablePublicFieldsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_decl_query()?,
        })
    }
}

impl Pipeline for MutablePublicFieldsPipeline {
    fn name(&self) -> &str {
        "mutable_public_fields"
    }

    fn description(&self) -> &str {
        "Detects public non-final fields — use getters/setters or make fields final"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let field_name_idx = find_capture_index(&self.field_query, "field_name");
        let field_decl_idx = find_capture_index(&self.field_query, "field_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_name_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(decl_node)) = (name_node, decl_node) {
                let is_public = has_modifier(decl_node, source, "public");
                let is_final = has_modifier(decl_node, source, "final");

                if is_public && !is_final {
                    let field_name = node_text(name_node, source);
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "mutable_public_field".to_string(),
                        message: format!(
                            "field `{field_name}` is public and non-final — use getters/setters or make it final"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MutablePublicFieldsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_mutable_public_field() {
        let src = "class Foo { public int x; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_public_field");
        assert!(findings[0].message.contains("`x`"));
    }

    #[test]
    fn clean_public_final() {
        let src = "class Foo { public final int x = 1; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_private() {
        let src = "class Foo { private int x; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
