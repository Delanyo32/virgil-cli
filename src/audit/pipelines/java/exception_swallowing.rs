use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_catch_clause_query, extract_snippet, find_capture_index, node_text};

pub struct ExceptionSwallowingPipeline {
    catch_query: Arc<Query>,
}

impl ExceptionSwallowingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            catch_query: compile_catch_clause_query()?,
        })
    }
}

impl Pipeline for ExceptionSwallowingPipeline {
    fn name(&self) -> &str {
        "exception_swallowing"
    }

    fn description(&self) -> &str {
        "Detects catch blocks that swallow exceptions (empty, printStackTrace-only, or return null)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.catch_query, tree.root_node(), source);

        let catch_body_idx = find_capture_index(&self.catch_query, "catch_body");
        let catch_idx = find_capture_index(&self.catch_query, "catch");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == catch_body_idx)
                .map(|c| c.node);
            let catch_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == catch_idx)
                .map(|c| c.node);

            if let (Some(body_node), Some(catch_node)) = (body_node, catch_node) {
                // Skip if catch block contains a logging call
                if catch_body_has_logging(body_node, source) {
                    continue;
                }

                let named_count = body_node.named_child_count();

                let (pattern, message) = if named_count == 0 {
                    (
                        "empty_catch",
                        "empty catch block silently swallows exception — log or rethrow instead",
                    )
                } else if named_count == 1 {
                    let child = body_node.named_child(0).unwrap();
                    if is_printstacktrace(child, source) {
                        (
                            "printstacktrace_only",
                            "catch block only calls printStackTrace() — use proper logging instead",
                        )
                    } else if is_return_null(child) {
                        (
                            "catch_return_null",
                            "catch block returns null — propagate the exception or return Optional",
                        )
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                let start = catch_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message: message.to_string(),
                    snippet: extract_snippet(source, catch_node, 3),
                });
            }
        }

        findings
    }
}

/// Check if a catch block body contains a logging method invocation.
/// Matches method names: log, warn, error, info, debug
/// Also matches if the receiver text contains "log" or "logger" (case-insensitive).
fn catch_body_has_logging(body_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut found = false;
    check_logging_recursive(body_node, source, &mut found);
    found
}

fn check_logging_recursive(node: tree_sitter::Node, source: &[u8], found: &mut bool) {
    if *found {
        return;
    }
    if node.kind() == "method_invocation" {
        // Check method name
        if let Some(name_node) = node.child_by_field_name("name") {
            let method_name = node_text(name_node, source);
            let log_methods = ["log", "warn", "error", "info", "debug"];
            if log_methods.contains(&method_name) {
                *found = true;
                return;
            }
        }
        // Check receiver text for log/logger
        if let Some(obj_node) = node.child_by_field_name("object") {
            let receiver = node_text(obj_node, source).to_lowercase();
            if receiver.contains("log") || receiver.contains("logger") {
                *found = true;
                return;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        check_logging_recursive(child, source, found);
    }
}

fn is_printstacktrace(node: tree_sitter::Node, source: &[u8]) -> bool {
    // expression_statement > method_invocation > name == "printStackTrace"
    if node.kind() != "expression_statement" {
        return false;
    }
    let Some(expr) = node.named_child(0) else {
        return false;
    };
    if expr.kind() != "method_invocation" {
        return false;
    }
    let Some(name) = expr.child_by_field_name("name") else {
        return false;
    };
    name.utf8_text(source).unwrap_or("") == "printStackTrace"
}

fn is_return_null(node: tree_sitter::Node) -> bool {
    if node.kind() != "return_statement" {
        return false;
    }
    // Check if the return value is null_literal
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            if child.kind() == "null_literal" {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExceptionSwallowingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_empty_catch() {
        let src = "class Foo { void m() { try { } catch (Exception e) { } } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_catch");
    }

    #[test]
    fn detects_printstacktrace_only() {
        let src =
            "class Foo { void m() { try { } catch (Exception e) { e.printStackTrace(); } } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "printstacktrace_only");
    }

    #[test]
    fn detects_return_null_catch() {
        let src =
            "class Foo { Object m() { try { return new Object(); } catch (Exception e) { return null; } } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "catch_return_null");
    }

    #[test]
    fn clean_logging_catch() {
        let src = "class Foo { void m() { try { } catch (Exception e) { logger.error(e); } } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_rethrow_catch() {
        let src = "class Foo { void m() { try { } catch (Exception e) { throw new RuntimeException(e); } } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
