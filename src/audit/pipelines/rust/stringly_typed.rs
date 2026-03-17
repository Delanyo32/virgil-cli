use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use super::primitives;

const SUSPICIOUS_NAMES: &[&str] = &[
    "kind", "type", "status", "mode", "state", "action", "level", "category", "role", "variant",
    "phase", "stage",
];

const STRING_TYPES: &[&str] = &["String", "&str", "Option<String>", "Option<&str>"];

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

    fn check_fields(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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
                    let start = field_cap.node.start_position();
                    let snippet = field_cap.node.utf8_text(source).unwrap_or("").to_string();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "stringly_typed_field".to_string(),
                        message: format!(
                            "field `{name}` has type `{type_text}` — consider using an enum instead of a string for type safety"
                        ),
                        snippet,
                    });
                }
            }
        }

        findings
    }

    fn check_params(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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

impl Pipeline for StringlyTypedPipeline {
    fn name(&self) -> &str {
        "stringly_typed"
    }

    fn description(&self) -> &str {
        "Detects struct fields and function parameters with string types that likely represent a fixed set of values and should be enums"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = self.check_fields(tree, source, file_path);
        findings.extend(self.check_params(tree, source, file_path));
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
        pipeline.check(&tree, source.as_bytes(), "test.rs")
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
}
