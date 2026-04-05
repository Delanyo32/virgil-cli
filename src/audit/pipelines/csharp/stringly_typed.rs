use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_field_decl_query, compile_parameter_query, compile_property_decl_query,
    extract_snippet, find_capture_index, is_csharp_suppressed, is_dto_or_data_class, node_text,
};

const SUSPICIOUS_NAMES: &[&str] = &[
    "status",
    "state",
    "type",
    "kind",
    "role",
    "mode",
    "category",
    "level",
    "priority",
    "stage",
    "phase",
    "action",
    "event_type",
    "permission",
    "color",
    "currency",
    "country",
    "payment_method",
    "http_method",
    "error_code",
    "day_of_week",
    "gender",
    "order_state",
    "user_role",
    "direction",
    "environment",
    "protocol",
    "encoding",
];

pub struct StringlyTypedPipeline {
    param_query: Arc<Query>,
    field_query: Arc<Query>,
    property_query: Arc<Query>,
}

impl StringlyTypedPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            param_query: compile_parameter_query()?,
            field_query: compile_field_decl_query()?,
            property_query: compile_property_decl_query()?,
        })
    }
}

impl GraphPipeline for StringlyTypedPipeline {
    fn name(&self) -> &str {
        "stringly_typed"
    }

    fn description(&self) -> &str {
        "Detects string-typed parameters, fields, and properties with names suggesting an enum"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check parameters
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.param_query, tree.root_node(), source);
            let param_type_idx = find_capture_index(&self.param_query, "param_type");
            let param_name_idx = find_capture_index(&self.param_query, "param_name");
            let param_idx = find_capture_index(&self.param_query, "param");

            while let Some(m) = matches.next() {
                let type_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_type_idx)
                    .map(|c| c.node);
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_name_idx)
                    .map(|c| c.node);
                let param_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == param_idx)
                    .map(|c| c.node);

                if let (Some(type_node), Some(name_node), Some(param_node)) =
                    (type_node, name_node, param_node)
                {
                    let type_text = node_text(type_node, source);
                    let param_name = node_text(name_node, source);

                    if !is_string_type(type_text) || !is_suspicious_name(param_name) {
                        continue;
                    }

                    // Skip if inside a DTO/data class
                    if is_in_dto_class(param_node, source, file_path) {
                        continue;
                    }

                    if is_csharp_suppressed(source, param_node, "stringly_typed") {
                        continue;
                    }

                    let start = param_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "stringly_typed".to_string(),
                        message: format!(
                            "parameter `{param_name}` is string-typed but its name suggests an enum \u{2014} consider a strongly-typed alternative"
                        ),
                        snippet: extract_snippet(source, param_node, 3),
                    });
                }
            }
        }

        // Check fields
        {
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
                    let field_name = node_text(name_node, source);
                    let type_text = get_field_type(decl_node, source).unwrap_or("");

                    if !is_string_type(type_text) || !is_suspicious_name(field_name) {
                        continue;
                    }

                    if is_in_dto_class(decl_node, source, file_path) {
                        continue;
                    }

                    if is_csharp_suppressed(source, decl_node, "stringly_typed") {
                        continue;
                    }

                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "stringly_typed".to_string(),
                        message: format!(
                            "field `{field_name}` is string-typed but its name suggests an enum \u{2014} consider a strongly-typed alternative"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
                    });
                }
            }
        }

        // Check properties
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.property_query, tree.root_node(), source);
            let prop_type_idx = find_capture_index(&self.property_query, "prop_type");
            let prop_name_idx = find_capture_index(&self.property_query, "prop_name");
            let prop_decl_idx = find_capture_index(&self.property_query, "prop_decl");

            while let Some(m) = matches.next() {
                let type_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == prop_type_idx)
                    .map(|c| c.node);
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == prop_name_idx)
                    .map(|c| c.node);
                let decl_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == prop_decl_idx)
                    .map(|c| c.node);

                if let (Some(type_node), Some(name_node), Some(decl_node)) =
                    (type_node, name_node, decl_node)
                {
                    let type_text = node_text(type_node, source);
                    let prop_name = node_text(name_node, source);

                    if !is_string_type(type_text) || !is_suspicious_name(prop_name) {
                        continue;
                    }

                    if is_in_dto_class(decl_node, source, file_path) {
                        continue;
                    }

                    if is_csharp_suppressed(source, decl_node, "stringly_typed") {
                        continue;
                    }

                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "stringly_typed".to_string(),
                        message: format!(
                            "property `{prop_name}` is string-typed but its name suggests an enum \u{2014} consider a strongly-typed alternative"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
                    });
                }
            }
        }

        findings
    }
}

fn is_string_type(type_text: &str) -> bool {
    matches!(type_text, "string" | "String" | "string?" | "String?")
}

fn is_suspicious_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    SUSPICIOUS_NAMES.iter().any(|&s| {
        lower == s
            || lower == s.replace('_', "")
            || lower.ends_with(&format!("_{s}"))
            || name.ends_with(&capitalize(s))
            || name.ends_with(&to_pascal_case(s))
    })
}

fn get_field_type<'a>(field_decl: tree_sitter::Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    let mut cursor = field_decl.walk();
    for child in field_decl.children(&mut cursor) {
        if child.kind() == "variable_declaration"
            && let Some(type_node) = child.child_by_field_name("type")
        {
            return Some(node_text(type_node, source));
        }
    }
    None
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split('_').map(|part| capitalize(part)).collect()
}

fn is_in_dto_class(node: tree_sitter::Node, source: &[u8], file_path: &str) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "class_declaration" {
            if let Some(name_node) = n.child_by_field_name("name") {
                let class_name = node_text(name_node, source);
                return is_dto_or_data_class(class_name, file_path);
            }
        }
        current = n.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "Service.cs")
    }

    fn parse_and_check_with_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = StringlyTypedPipeline::new().unwrap();
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
    fn detects_string_status_param() {
        let src = r#"
class Foo {
    void SetStatus(string status) { }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "stringly_typed");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn detects_string_type_property() {
        let src = r#"
class Foo {
    public string Role { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_normal_string_param() {
        let src = r#"
class Foo {
    void SetName(string name) { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_enum_typed() {
        let src = r#"
class Foo {
    void SetStatus(Status status) { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_compound_name() {
        let src = r#"
class Foo {
    void SetState(string orderStatus) { }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn dto_class_excluded() {
        let src = r#"
class OrderDto {
    public string Status { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nullable_string_detected() {
        let src = r#"
class Foo {
    void M(string? status) { }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn expanded_names_detected() {
        let src = r#"
class Foo {
    void M(string paymentMethod) { }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn test_file_excluded() {
        let src = r#"
class Foo {
    void SetStatus(string status) { }
}
"#;
        let findings = parse_and_check_with_path(src, "FooTests.cs");
        assert!(findings.is_empty());
    }
}
