use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_field_decl_query, extract_snippet, find_capture_index, has_modifier, node_text,
};

pub struct StaticGlobalStatePipeline {
    field_query: Arc<Query>,
}

impl StaticGlobalStatePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_decl_query()?,
        })
    }
}

impl Pipeline for StaticGlobalStatePipeline {
    fn name(&self) -> &str {
        "static_global_state"
    }

    fn description(&self) -> &str {
        "Detects mutable static fields that create hidden global state"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let field_decl_idx = find_capture_index(&self.field_query, "field_decl");
        let field_name_idx = find_capture_index(&self.field_query, "field_name");

        while let Some(m) = matches.next() {
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_decl_idx)
                .map(|c| c.node);
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_name_idx)
                .map(|c| c.node);

            if let (Some(decl_node), Some(name_node)) = (decl_node, name_node) {
                if has_modifier(decl_node, source, "static")
                    && !has_modifier(decl_node, source, "readonly")
                    && !has_modifier(decl_node, source, "const")
                {
                    let field_name = node_text(name_node, source);
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "mutable_static_field".to_string(),
                        message: format!(
                            "mutable static field `{field_name}` creates hidden global state \u{2014} consider readonly, const, or dependency injection"
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = StaticGlobalStatePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_mutable_static() {
        let src = r#"
class Foo {
    private static int _counter;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_static_field");
    }

    #[test]
    fn clean_static_readonly() {
        let src = r#"
class Foo {
    private static readonly int MaxSize = 100;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_const() {
        let src = r#"
class Foo {
    const int MaxSize = 100;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_instance_field() {
        let src = r#"
class Foo {
    private int _counter;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = "class Foo { public static string Instance; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "static_global_state");
    }
}
