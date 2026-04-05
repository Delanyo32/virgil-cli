use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_class_decl_query, extract_snippet, find_capture_index, has_csharp_attribute,
    is_csharp_suppressed, is_dto_or_data_class, node_text,
};

const MIN_PROPERTIES: usize = 3;

/// Methods considered trivial — their presence alone doesn't count as "behavior".
const TRIVIAL_METHOD_NAMES: &[&str] = &["ToString", "GetHashCode", "Equals", "CompareTo"];

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

impl GraphPipeline for AnemicDomainModelPipeline {
    fn name(&self) -> &str {
        "anemic_domain_model"
    }

    fn description(&self) -> &str {
        "Detects classes with only properties and no methods (excluding DTOs/ViewModels)"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

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

                // Skip DTO/data classes by name suffix or file path
                if is_dto_or_data_class(class_name, file_path) {
                    continue;
                }

                // Skip classes with ORM/serialization attributes
                if has_csharp_attribute(decl_node, source, "Table")
                    || has_csharp_attribute(decl_node, source, "DataContract")
                    || has_csharp_attribute(decl_node, source, "ProtoContract")
                {
                    continue;
                }

                if is_csharp_suppressed(source, decl_node, "anemic_domain_model") {
                    continue;
                }

                let mut property_count = 0;
                let mut nontrivial_method_count = 0;
                let mut body_cursor = body_node.walk();
                for child in body_node.children(&mut body_cursor) {
                    match child.kind() {
                        "property_declaration" => property_count += 1,
                        "method_declaration" => {
                            // Check if method is trivial (ToString, GetHashCode, etc.)
                            if let Some(name) = child.child_by_field_name("name") {
                                let method_name = node_text(name, source);
                                if !TRIVIAL_METHOD_NAMES.contains(&method_name) {
                                    nontrivial_method_count += 1;
                                }
                            } else {
                                nontrivial_method_count += 1;
                            }
                        }
                        _ => {}
                    }
                }

                if property_count >= MIN_PROPERTIES && nontrivial_method_count == 0 {
                    // Severity: 3-5 properties → info, 6+ → warning
                    let severity = if property_count >= 6 { "warning" } else { "info" };

                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "src/Domain/Order.cs")
    }

    fn parse_and_check_with_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = AnemicDomainModelPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
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

    #[test]
    fn entity_suffix_excluded() {
        let src = r#"
class OrderEntity {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
    public string Status { get; set; }
    public string Category { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn table_attribute_excluded() {
        let src = r#"
[Table("orders")]
class Order {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn models_directory_excluded() {
        let src = r#"
class Order {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
}
"#;
        let findings = parse_and_check_with_path(src, "src/Models/Order.cs");
        assert!(findings.is_empty());
    }

    #[test]
    fn one_trivial_method_still_anemic() {
        let src = r#"
class Order {
    public int Id { get; set; }
    public string Name { get; set; }
    public decimal Price { get; set; }
    public override string ToString() { return Name; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "anemic_class");
    }

    #[test]
    fn severity_graduation() {
        // 3 properties → info
        let src = r#"
class Foo {
    public int A { get; set; }
    public int B { get; set; }
    public int C { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "info");

        // 6 properties → warning
        let src = r#"
class Foo {
    public int A { get; set; }
    public int B { get; set; }
    public int C { get; set; }
    public int D { get; set; }
    public int E { get; set; }
    public int F { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
    }
}
