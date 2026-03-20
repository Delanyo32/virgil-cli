use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{self, extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

pub struct RaceConditionsPipeline {
    static_query: Arc<Query>,
}

impl RaceConditionsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            static_query: primitives::compile_static_item_query()?,
        })
    }
}

impl Pipeline for RaceConditionsPipeline {
    fn name(&self) -> &str {
        "race_conditions"
    }

    fn description(&self) -> &str {
        "Detects race condition risks: static mut declarations"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.static_query, tree.root_node(), source);

        let mut_idx = find_capture_index(&self.static_query, "mut_spec");
        let name_idx = find_capture_index(&self.static_query, "static_name");
        let item_idx = find_capture_index(&self.static_query, "static_item");

        while let Some(m) = matches.next() {
            let has_mut = m.captures.iter().any(|c| c.index as usize == mut_idx);

            if has_mut {
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == name_idx)
                    .map(|c| c.node);
                let item_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == item_idx)
                    .map(|c| c.node);

                if let (Some(name), Some(item)) = (name_node, item_node) {
                    let var_name = node_text(name, source);
                    let start = item.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "static_mut".to_string(),
                        message: format!(
                            "static mut `{}` is a data race hazard; consider AtomicXxx or Mutex",
                            var_name
                        ),
                        snippet: extract_snippet(source, item, 1),
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RaceConditionsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_static_mut() {
        let src = r#"static mut COUNTER: u32 = 0;"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "static_mut");
        assert!(findings[0].message.contains("COUNTER"));
    }

    #[test]
    fn ignores_static_immutable() {
        let src = r#"static COUNTER: u32 = 0;"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_static_mut() {
        let src = r#"
static mut A: u32 = 0;
static mut B: i64 = 0;
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }
}
