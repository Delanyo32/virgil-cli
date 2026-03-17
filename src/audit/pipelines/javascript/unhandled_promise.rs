use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::javascript_primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

pub struct UnhandledPromisePipeline {
    call_query: Arc<Query>,
}

impl UnhandledPromisePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }

    /// Check if this .then() call has a .catch() chained after it,
    /// or if .then() has 2+ arguments (second is error handler).
    fn is_handled(call_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Check if .then() has 2+ arguments (rejection handler)
        if let Some(args) = call_node.child_by_field_name("arguments") {
            if args.named_child_count() >= 2 {
                return true;
            }
        }

        // Check if .then() is the object of a .catch() or .finally() chain
        if let Some(parent) = call_node.parent() {
            if parent.kind() == "member_expression" {
                if let Some(prop) = parent.child_by_field_name("property") {
                    let prop_name = node_text(prop, source);
                    if prop_name == "catch" || prop_name == "finally" {
                        return true;
                    }
                }
            }
        }

        false
    }
}

impl Pipeline for UnhandledPromisePipeline {
    fn name(&self) -> &str {
        "unhandled_promise"
    }

    fn description(&self) -> &str {
        "Detects .then() calls without .catch() error handling"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(method), Some(call)) = (method_node, call_node) {
                let method_name = node_text(method.node, source);

                if method_name != "then" {
                    continue;
                }

                if Self::is_handled(call.node, source) {
                    continue;
                }

                let start = call.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unhandled_then".to_string(),
                    message: "`.then()` without `.catch()` — unhandled promise rejection"
                        .to_string(),
                    snippet: extract_snippet(source, call.node, 1),
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
        let pipeline = UnhandledPromisePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_then_without_catch() {
        let findings = parse_and_check("fetch(url).then(data => process(data));");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unhandled_then");
    }

    #[test]
    fn skips_then_with_catch() {
        let findings =
            parse_and_check("fetch(url).then(data => process(data)).catch(err => handle(err));");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_then_with_two_args() {
        let findings = parse_and_check("fetch(url).then(onSuccess, onError);");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_then_methods() {
        let findings = parse_and_check("obj.map(x => x * 2);");
        assert!(findings.is_empty());
    }
}
