use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::php_primitives::{compile_error_suppression_query, extract_snippet, find_capture_index};

pub struct ErrorSuppressionPipeline {
    suppress_query: Arc<Query>,
}

impl ErrorSuppressionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            suppress_query: compile_error_suppression_query()?,
        })
    }
}

impl Pipeline for ErrorSuppressionPipeline {
    fn name(&self) -> &str {
        "error_suppression"
    }

    fn description(&self) -> &str {
        "Detects use of the @ error suppression operator"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.suppress_query, tree.root_node(), source);

        let suppress_idx = find_capture_index(&self.suppress_query, "suppress");

        while let Some(m) = matches.next() {
            let cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == suppress_idx);

            if let Some(cap) = cap {
                let node = cap.node;
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "at_operator".to_string(),
                    message: "error suppression operator @ hides failures — use proper error handling".to_string(),
                    snippet: extract_snippet(source, node, 2),
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ErrorSuppressionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_at_operator() {
        let src = "<?php\n@file_get_contents('x');\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "at_operator");
    }

    #[test]
    fn detects_multiple_suppressions() {
        let src = "<?php\n@fopen('a', 'r');\n@unlink('b');\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn clean_no_suppression() {
        let src = "<?php\nfile_get_contents('x');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
