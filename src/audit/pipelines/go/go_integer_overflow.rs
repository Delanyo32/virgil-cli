use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{self, extract_snippet, find_capture_index, node_text};

const NARROWING_TYPES: &[&str] = &["int8", "int16", "int32", "uint8", "uint16", "uint32"];

pub struct GoIntegerOverflowPipeline {
    conversion_query: Arc<Query>,
}

impl GoIntegerOverflowPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            conversion_query: primitives::compile_type_conversion_query()?,
        })
    }
}

impl Pipeline for GoIntegerOverflowPipeline {
    fn name(&self) -> &str {
        "integer_overflow"
    }

    fn description(&self) -> &str {
        "Detects integer overflow risks: narrowing type conversions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.conversion_query, tree.root_node(), source);

        let type_name_idx = find_capture_index(&self.conversion_query, "type_name");
        let conversion_idx = find_capture_index(&self.conversion_query, "conversion");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == type_name_idx)
                .map(|c| c.node);
            let conversion_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == conversion_idx)
                .map(|c| c.node);

            if let (Some(type_node), Some(conversion_node)) = (type_node, conversion_node) {
                let type_name = node_text(type_node, source);

                if !NARROWING_TYPES.contains(&type_name) {
                    continue;
                }

                let start = conversion_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "narrowing_conversion".to_string(),
                    message: format!("narrowing conversion to {type_name} may overflow"),
                    snippet: extract_snippet(source, conversion_node, 1),
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
        let pipeline = GoIntegerOverflowPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_int32_conversion() {
        let src = r#"package main

func f() {
	var x int64
	_ = int32(x)
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "narrowing_conversion");
    }

    #[test]
    fn detects_uint8_conversion() {
        let src = r#"package main

func f() {
	var x int
	_ = uint8(x)
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "narrowing_conversion");
    }

    #[test]
    fn ignores_int64() {
        let src = r#"package main

func f() {
	var x int32
	_ = int64(x)
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_string_conversion() {
        let src = r#"package main

func f() {
	var x int
	_ = string(x)
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
