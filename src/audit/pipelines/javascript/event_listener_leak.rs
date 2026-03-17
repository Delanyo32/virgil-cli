use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::javascript_primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

pub struct EventListenerLeakPipeline {
    call_query: Arc<Query>,
}

impl EventListenerLeakPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

impl Pipeline for EventListenerLeakPipeline {
    fn name(&self) -> &str {
        "event_listener_leak"
    }

    fn description(&self) -> &str {
        "Detects addEventListener calls without corresponding removeEventListener"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        let mut add_calls = Vec::new();
        let mut has_remove = false;

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(method), Some(call)) = (method_node, call_node) {
                let method_name = node_text(method.node, source);
                match method_name {
                    "addEventListener" => {
                        add_calls.push(call.node);
                    }
                    "removeEventListener" => {
                        has_remove = true;
                    }
                    _ => {}
                }
            }
        }

        if has_remove || add_calls.is_empty() {
            return Vec::new();
        }

        add_calls
            .into_iter()
            .map(|node| {
                let start = node.start_position();
                AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "missing_remove_listener".to_string(),
                    message:
                        "addEventListener without removeEventListener — potential memory leak"
                            .to_string(),
                    snippet: extract_snippet(source, node, 1),
                }
            })
            .collect()
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
        let pipeline = EventListenerLeakPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_add_without_remove() {
        let src = "element.addEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_remove_listener");
    }

    #[test]
    fn skips_when_remove_present() {
        let src = "element.addEventListener('click', handler);\nelement.removeEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_findings_without_add() {
        let src = "element.removeEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
