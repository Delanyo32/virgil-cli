use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_class_decl_query, extract_snippet, find_capture_index, node_text};

const MIN_PROPERTIES: usize = 3;
const EXCLUDED_SUFFIXES: &[&str] = &[
    "Dto",
    "DTO",
    "ViewModel",
    "Request",
    "Response",
    "Command",
    "Query",
    "Event",
    "Message",
    "Options",
    "Settings",
    "Config",
    "Configuration",
];

pub struct AnemicDomainModelPipeline {
    class_query: Arc<Query>,
}

impl AnemicDomainModelPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_decl_query()?,
        })
    }
}

impl Pipeline for AnemicDomainModelPipeline {
    fn name(&self) -> &str {
        "anemic_domain_model"
    }

    fn description(&self) -> &str {
        "Detects classes with only properties and no methods (excluding DTOs/ViewModels)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let class_name_idx = find_capture_index(&self.class_query, "class_name");
        let class_body_idx = find_capture_index(&self.class_query, "class_body");
        let class_decl_idx = find_capture_index(&self.class_query, "class_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_body_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(decl_node)) =
                (name_node, body_node, decl_node)
            {
                let class_name = node_text(name_node, source);

                // Skip excluded suffixes
                if EXCLUDED_SUFFIXES.iter().any(|s| class_name.ends_with(s)) {
                    continue;
                }

                let mut property_count = 0;
                let mut method_count = 0;
                let mut body_cursor = body_node.walk();
                for child in body_node.children(&mut body_cursor) {
                    match child.kind() {
                        "property_declaration" => property_count += 1,
                        "method_declaration" => method_count += 1,
                        _ => {}
                    }
                }

                if property_count >= MIN_PROPERTIES && method_count == 0 {
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "anemic_class".to_string(),
                        message: format!(
                            "class `{class_name}` has {property_count} properties but no methods \u{2014} consider adding behavior or making it a record/DTO"
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
        let pipeline = AnemicDomainModelPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_anemic_class() {
        let src = r#"
class Order {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "anemic_class");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn clean_class_with_methods() {
        let src = r#"
class Order {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
    public decimal CalculateTotal() { return Price; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_dto_excluded() {
        let src = r#"
class OrderDto {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_viewmodel_excluded() {
        let src = r#"
class OrderViewModel {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_small_class() {
        let src = r#"
class Point {
    public int X { get; set; }
    public int Y { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty()); // only 2 properties < 3 threshold
    }
}
