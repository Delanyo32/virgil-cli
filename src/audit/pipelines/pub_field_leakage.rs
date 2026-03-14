use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::primitives;
use crate::language::Language;

const PUB_FIELD_THRESHOLD: usize = 3;

pub struct PubFieldLeakagePipeline {
    struct_query: Arc<Query>,
}

impl PubFieldLeakagePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            struct_query: primitives::compile_struct_fields_query(Language::Rust)?,
        })
    }
}

impl Pipeline for PubFieldLeakagePipeline {
    fn name(&self) -> &str {
        "pub_field_leakage"
    }

    fn description(&self) -> &str {
        "Detects structs with many public fields that leak implementation details — consider using accessor methods"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.struct_query, tree.root_node(), source);

        let name_idx = self
            .struct_query
            .capture_names()
            .iter()
            .position(|n| *n == "struct_name")
            .unwrap();
        let fields_idx = self
            .struct_query
            .capture_names()
            .iter()
            .position(|n| *n == "fields")
            .unwrap();
        let struct_idx = self
            .struct_query
            .capture_names()
            .iter()
            .position(|n| *n == "struct_def")
            .unwrap();

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == name_idx);
            let fields_node = m.captures.iter().find(|c| c.index as usize == fields_idx);
            let struct_node = m.captures.iter().find(|c| c.index as usize == struct_idx);

            if let (Some(name_cap), Some(fields_cap), Some(struct_cap)) =
                (name_node, fields_node, struct_node)
            {
                let pub_count = (0..fields_cap.node.named_child_count())
                    .filter_map(|i| fields_cap.node.named_child(i))
                    .filter(|child| child.kind() == "field_declaration")
                    .filter(|child| {
                        (0..child.named_child_count())
                            .filter_map(|i| child.named_child(i))
                            .any(|c| c.kind() == "visibility_modifier")
                    })
                    .count();

                if pub_count > PUB_FIELD_THRESHOLD {
                    let name = name_cap.node.utf8_text(source).unwrap_or("");
                    let start = struct_cap.node.start_position();
                    let snippet =
                        primitives::extract_snippet(source, struct_cap.node, 3);
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "pub_field_leakage".to_string(),
                        message: format!(
                            "struct `{name}` has {pub_count} public fields (threshold: {PUB_FIELD_THRESHOLD}) — consider using accessor methods to encapsulate"
                        ),
                        snippet,
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PubFieldLeakagePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_many_pub_fields() {
        let src = r#"
struct Leaky {
    pub a: i32,
    pub b: String,
    pub c: Vec<u8>,
    pub d: bool,
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "pub_field_leakage");
        assert!(findings[0].message.contains("4 public fields"));
    }

    #[test]
    fn skips_few_pub_fields() {
        let src = r#"
struct Ok {
    pub a: i32,
    pub b: String,
    c: Vec<u8>,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_all_private_fields() {
        let src = r#"
struct Private {
    a: i32,
    b: String,
    c: Vec<u8>,
    d: bool,
    e: f64,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
