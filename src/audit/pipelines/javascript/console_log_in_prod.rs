use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::{is_nolint_suppressed, is_test_file};

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

    /// Graduate severity by console method.
    fn severity_for_method(method: &str) -> &'static str {
        match method {
            "log" | "debug" | "trace" => "warning",
            _ => "info", // warn, error, info
        }
    }

    /// Check if any ancestor of the node is a catch_clause.
    fn is_inside_catch(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "catch_clause" {
                return true;
            }
            current = parent.parent();
        }
        false
    }
}

impl NodePipeline for ConsoleLogPipeline {
    fn name(&self) -> &str {
        "console_log"
    }

    fn description(&self) -> &str {
        "Detects console.log/warn/error/debug/info/trace calls left in code"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        // Suppress entire file if it's a test file
        if is_test_file(file_path) {
            return Vec::new();
        }

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

                if obj_name != "console" || !CONSOLE_METHODS.contains(&method_name) {
                    continue;
                }

                // Suppress console.error inside catch blocks (legitimate error reporting)
                if method_name == "error" && Self::is_inside_catch(call.node) {
                    continue;
                }

                if is_nolint_suppressed(source, call.node, self.name()) {
                    continue;
                }

                let severity = Self::severity_for_method(method_name);
                let start = call.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "console_log".to_string(),
                    message: format!(
                        "`console.{method_name}()` should be removed or replaced with a proper logger"
                    ),
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
        parse_and_check_path(source, "test.js")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ConsoleLogPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
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

    // --- New tests ---

    #[test]
    fn test_file_suppressed() {
        let findings = parse_and_check_path("console.log('debug');", "src/foo.test.js");
        assert!(findings.is_empty());
    }

    #[test]
    fn console_error_in_catch_suppressed() {
        let findings = parse_and_check("try {} catch(e) { console.error(e); }");
        assert!(findings.is_empty());
    }

    #[test]
    fn severity_graduation_log() {
        let findings = parse_and_check("console.log('x');");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn severity_graduation_error() {
        let findings = parse_and_check("console.error('x');");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn severity_graduation_debug() {
        let findings = parse_and_check("console.debug('x');");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn nolint_suppresses() {
        let findings = parse_and_check("// NOLINT(console_log)\nconsole.log('x');");
        assert!(findings.is_empty());
    }
}
