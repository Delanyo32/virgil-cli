use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_function_call_query, extract_snippet, find_capture_index, node_text,
};

pub struct ExtractUsagePipeline {
    call_query: Arc<Query>,
}

impl ExtractUsagePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_function_call_query()?,
        })
    }
}

impl Pipeline for ExtractUsagePipeline {
    fn name(&self) -> &str {
        "extract_usage"
    }

    fn description(&self) -> &str {
        "Detects use of extract() which pollutes the local scope with untracked variables"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(call_node)) = (name_node, call_node) {
                let fn_name = node_text(name_node, source);
                if fn_name == "extract" {
                    let start = call_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "extract_call".to_string(),
                        message: "extract() pollutes the local scope with untracked variables — use explicit array access instead".to_string(),
                        snippet: extract_snippet(source, call_node, 2),
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExtractUsagePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_extract() {
        let src = "<?php\nextract($_POST);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "extract_call");
    }

    #[test]
    fn clean_no_extract() {
        let src = "<?php\n$name = $_POST['name'];\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_other_functions() {
        let src = "<?php\ncompact('a', 'b');\narray_merge($a, $b);\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
