use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::csharp_primitives::{compile_class_decl_query, extract_snippet, find_capture_index, node_text};

const MAX_METHODS: usize = 10;
const MAX_FIELDS: usize = 15;

pub struct GodClassPipeline {
    class_query: Arc<Query>,
}

impl GodClassPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_decl_query()?,
        })
    }
}

impl Pipeline for GodClassPipeline {
    fn name(&self) -> &str {
        "god_class"
    }

    fn description(&self) -> &str {
        "Detects classes with too many methods or fields"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let class_name_idx = find_capture_index(&self.class_query, "class_name");
        let class_body_idx = find_capture_index(&self.class_query, "class_body");
        let class_decl_idx = find_capture_index(&self.class_query, "class_decl");

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == class_name_idx).map(|c| c.node);
            let body_node = m.captures.iter().find(|c| c.index as usize == class_body_idx).map(|c| c.node);
            let decl_node = m.captures.iter().find(|c| c.index as usize == class_decl_idx).map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(decl_node)) = (name_node, body_node, decl_node) {
                let class_name = node_text(name_node, source);

                let mut method_count = 0;
                let mut field_count = 0;
                let mut body_cursor = body_node.walk();
                for child in body_node.children(&mut body_cursor) {
                    match child.kind() {
                        "method_declaration" | "constructor_declaration" => method_count += 1,
                        "field_declaration" => field_count += 1,
                        _ => {}
                    }
                }

                if method_count > MAX_METHODS {
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "too_many_methods".to_string(),
                        message: format!(
                            "class `{class_name}` has {method_count} methods (>{MAX_METHODS}) \u{2014} consider splitting into smaller classes"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
                    });
                }

                if field_count > MAX_FIELDS {
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "too_many_fields".to_string(),
                        message: format!(
                            "class `{class_name}` has {field_count} fields (>{MAX_FIELDS}) \u{2014} consider splitting into smaller classes"
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
        parser.set_language(&Language::CSharp.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GodClassPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_too_many_methods() {
        let methods: Vec<String> = (0..12).map(|i| format!("public void M{i}() {{ }}")).collect();
        let src = format!("class BigClass {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "too_many_methods");
    }

    #[test]
    fn detects_too_many_fields() {
        let fields: Vec<String> = (0..16).map(|i| format!("private int f{i};")).collect();
        let src = format!("class BigClass {{ {} }}", fields.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "too_many_fields");
    }

    #[test]
    fn clean_small_class() {
        let src = r#"
class SmallClass {
    private int _x;
    public void DoWork() { }
    public void DoMore() { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let methods: Vec<String> = (0..11).map(|i| format!("void M{i}() {{ }}")).collect();
        let src = format!("class Foo {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings[0].severity, "warning");
        assert_eq!(findings[0].pipeline, "god_class");
    }
}
