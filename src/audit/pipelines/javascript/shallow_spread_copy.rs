use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_spread_in_object_query, extract_snippet, find_capture_index};

pub struct ShallowSpreadCopyPipeline {
    spread_query: Arc<Query>,
}

impl ShallowSpreadCopyPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            spread_query: compile_spread_in_object_query()?,
        })
    }
}

impl Pipeline for ShallowSpreadCopyPipeline {
    fn name(&self) -> &str {
        "shallow_spread_copy"
    }

    fn description(&self) -> &str {
        "Detects `{ ...obj }` shallow copies that may not deeply clone nested objects"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.spread_query, tree.root_node(), source);

        let target_idx = find_capture_index(&self.spread_query, "target");
        let obj_idx = find_capture_index(&self.spread_query, "obj");

        while let Some(m) = matches.next() {
            let target_cap = m.captures.iter().find(|c| c.index as usize == target_idx);
            let obj_cap = m.captures.iter().find(|c| c.index as usize == obj_idx);

            if let (Some(target), Some(obj)) = (target_cap, obj_cap) {
                // Only flag when spread target is a plain identifier (variable reference)
                // Skip function calls like { ...getDefaults() } which produce fresh objects
                if target.node.kind() != "identifier" {
                    continue;
                }

                let start = obj.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "shallow_spread_copy".to_string(),
                    message: "spread copy is shallow — nested objects are still shared references"
                        .to_string(),
                    snippet: extract_snippet(source, obj.node, 1),
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ShallowSpreadCopyPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_spread_of_identifier() {
        let findings = parse_and_check("let copy = { ...obj };");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "shallow_spread_copy");
    }

    #[test]
    fn skips_spread_of_call() {
        let findings = parse_and_check("let copy = { ...getDefaults() };");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_spread_no_findings() {
        let findings = parse_and_check("let obj = { a: 1, b: 2 };");
        assert!(findings.is_empty());
    }
}
