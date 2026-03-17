use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

const CONSOLE_METHODS: &[&str] = &["log", "warn", "error", "debug", "info", "trace"];

pub struct ConsoleLogPipeline {
    call_query: Arc<Query>,
}

impl ConsoleLogPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

impl Pipeline for ConsoleLogPipeline {
    fn name(&self) -> &str {
        "console_log"
    }

    fn description(&self) -> &str {
        "Detects console.log/warn/error/debug/info/trace calls left in code"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let obj_idx = find_capture_index(&self.call_query, "obj");
        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let obj_node = m.captures.iter().find(|c| c.index as usize == obj_idx);
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(obj), Some(method), Some(call)) = (obj_node, method_node, call_node) {
                let obj_name = node_text(obj.node, source);
                let method_name = node_text(method.node, source);

                if obj_name == "console" && CONSOLE_METHODS.contains(&method_name) {
                    let start = call.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "console_log".to_string(),
                        message: format!(
                            "`console.{method_name}()` should be removed or replaced with a proper logger"
                        ),
                        snippet: extract_snippet(source, call.node, 1),
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ConsoleLogPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_console_log() {
        let findings = parse_and_check("console.log('debug');");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "console_log");
    }

    #[test]
    fn detects_console_warn() {
        let findings = parse_and_check("console.warn('warning');");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_console_error() {
        let findings = parse_and_check("console.error('err');");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_other_objects() {
        let findings = parse_and_check("logger.log('msg');");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_other_methods() {
        let findings = parse_and_check("console.clear();");
        assert!(findings.is_empty());
    }
}
