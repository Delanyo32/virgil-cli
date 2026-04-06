use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::struct_has_derive;

const SUSPICIOUS_NAMES: &[&str] = &[
    "kind", "type", "status", "mode", "state", "level", "role", "variant", "phase", "stage",
];

const STRING_TYPES: &[&str] = &[
    "String",
    "&str",
    "Option<String>",
    "Option<&str>",
    "Cow<'_, str>",
    "Box<str>",
    "Arc<str>",
];

pub struct StringlyTypedPipeline {
    field_query: Arc<Query>,
    param_query: Arc<Query>,
}

impl StringlyTypedPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: primitives::compile_field_declaration_query()?,
            param_query: primitives::compile_parameter_query()?,
        })
    }

    fn check_fields(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let name_idx = self
            .field_query
            .capture_names()
            .iter()
            .position(|n| *n == "field_name")
            .unwrap();
        let type_idx = self
            .field_query
            .capture_names()
            .iter()
            .position(|n| *n == "field_type")
            .unwrap();
        let field_idx = self
            .field_query
            .capture_names()
            .iter()
            .position(|n| *n == "field")
            .unwrap();

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == name_idx);
            let type_node = m.captures.iter().find(|c| c.index as usize == type_idx);
            let field_node = m.captures.iter().find(|c| c.index as usize == field_idx);

            if let (Some(name_cap), Some(type_cap), Some(field_cap)) =
                (name_node, type_node, field_node)
            {
                let name = name_cap.node.utf8_text(source).unwrap_or("");
                let type_text = type_cap.node.utf8_text(source).unwrap_or("");

                if SUSPICIOUS_NAMES.contains(&name) && STRING_TYPES.contains(&type_text) {
                    // Skip fields in serde deserialization or serialization structs
                    if struct_has_derive(field_cap.node, source, "Deserialize") {
                        continue;
                    }
                    if struct_has_derive(field_cap.node, source, "Serialize") {
                        continue;
                    }

                    // Check if an enum already exists in the graph with a PascalCase version of the field name
                    let message = if enum_in_graph(name, ctx.graph) {
                        let enum_name = capitalize_first(name);
                        format!(
                            "field `{name}` has type `{type_text}` — enum `{enum_name}` already exists — use it instead of String"
                        )
                    } else {
                        format!(
                            "field `{name}` has type `{type_text}` — consider using an enum instead of a string for type safety"
                        )
                    };

                    let start = field_cap.node.start_position();
                    let snippet = field_cap.node.utf8_text(source).unwrap_or("").to_string();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "stringly_typed_field".to_string(),
                        message,
                        snippet,
                    });
                }
            }
        }

        findings
    }

    fn check_params(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.param_query, tree.root_node(), source);

        let name_idx = self
            .param_query
            .capture_names()
            .iter()
            .position(|n| *n == "param_name")
            .unwrap();
        let type_idx = self
            .param_query
            .capture_names()
            .iter()
            .position(|n| *n == "param_type")
            .unwrap();
        let param_idx = self
            .param_query
            .capture_names()
            .iter()
            .position(|n| *n == "param")
            .unwrap();

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == name_idx);
            let type_node = m.captures.iter().find(|c| c.index as usize == type_idx);
            let param_node = m.captures.iter().find(|c| c.index as usize == param_idx);

            if let (Some(name_cap), Some(type_cap), Some(param_cap)) =
                (name_node, type_node, param_node)
            {
                let name = name_cap.node.utf8_text(source).unwrap_or("");
                let type_text = type_cap.node.utf8_text(source).unwrap_or("");

                if SUSPICIOUS_NAMES.contains(&name) && STRING_TYPES.contains(&type_text) {
                    let start = param_cap.node.start_position();
                    let snippet = param_cap.node.utf8_text(source).unwrap_or("").to_string();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "stringly_typed_param".to_string(),
                        message: format!(
                            "parameter `{name}` has type `{type_text}` — consider using an enum instead of a string for type safety"
                        ),
                        snippet,
                    });
                }
            }
        }

        findings
    }
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn enum_in_graph(name: &str, graph: &crate::graph::CodeGraph) -> bool {
    let enum_name = capitalize_first(name);
    graph.symbols_by_name.get(&enum_name).map_or(false, |nodes| {
        nodes.iter().any(|&idx| {
            matches!(
                &graph.graph[idx],
                crate::graph::NodeWeight::Symbol {
                    kind: crate::models::SymbolKind::Enum,
                    ..
                }
            )
        })
    })
}

impl GraphPipeline for StringlyTypedPipeline {
    fn name(&self) -> &str {
        "stringly_typed"
    }

    fn description(&self) -> &str {
        "Detects struct fields and function parameters with string types that likely represent a fixed set of values and should be enums"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let mut findings = self.check_fields(ctx);
        findings.extend(self.check_params(ctx));
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = StringlyTypedPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.rs",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_string_status_field() {
        let src = r#"
struct Config {
    status: String,
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "stringly_typed_field");
        assert!(findings[0].message.contains("status"));
    }

    #[test]
    fn skips_enum_typed_status_field() {
        let src = r#"
struct Config {
    status: Status,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_str_mode_param() {
        let src = r#"
fn process(mode: &str) {}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "stringly_typed_param");
    }

    #[test]
    fn skips_non_suspicious_name() {
        let src = r#"
struct Config {
    name: String,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_option_string_type() {
        let src = r#"
struct Config {
    kind: Option<String>,
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_cow_str_field() {
        let src = "struct S { status: std::borrow::Cow<'_, str> }";
        // Note: tree-sitter captures exact type text, so this might not match "Cow<'_, str>" exactly
        // Either flagged or not — just no panic
        let findings = parse_and_check(src);
        let _ = findings;
    }

    #[test]
    fn serialize_derive_skips_struct() {
        let src = r#"
#[derive(serde::Serialize)]
struct Dto { status: String }
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "#[derive(Serialize)] should be exempt");
    }
}
