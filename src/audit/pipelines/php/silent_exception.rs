use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::{is_nolint_suppressed, is_test_file};

use super::primitives::{
    compile_catch_clause_query, extract_snippet, find_capture_index, node_text,
};

/// Broad exception types that catch everything -- higher severity when empty.
const BROAD_EXCEPTION_TYPES: &[&str] = &[
    "Exception",
    "\\Exception",
    "Throwable",
    "\\Throwable",
];

pub struct SilentExceptionPipeline {
    catch_query: Arc<Query>,
}

impl SilentExceptionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            catch_query: compile_catch_clause_query()?,
        })
    }
}

/// Check if the catch type list contains a broad exception type (Exception/Throwable).
fn catches_broad_exception(type_node: tree_sitter::Node, source: &[u8]) -> bool {
    for i in 0..type_node.named_child_count() {
        if let Some(child) = type_node.named_child(i) {
            let type_text = node_text(child, source);
            if BROAD_EXCEPTION_TYPES.contains(&type_text) {
                return true;
            }
        }
    }
    false
}

/// Classify the catch body as empty, trivial, or substantive.
#[derive(Debug, PartialEq)]
enum CatchBodyKind {
    Empty,
    ReturnOnly,
    TrivialReturn, // return false; return null;
    ContinueOnly,
    AssignmentOnly,
    Substantive,
}

fn classify_body(body_node: tree_sitter::Node, source: &[u8]) -> CatchBodyKind {
    let named_count = body_node.named_child_count();
    if named_count == 0 {
        return CatchBodyKind::Empty;
    }

    if named_count == 1 {
        if let Some(child) = body_node.named_child(0) {
            match child.kind() {
                "return_statement" => {
                    // Check if it's return false; or return null;
                    if child.named_child_count() == 1 {
                        if let Some(val) = child.named_child(0) {
                            let val_text = node_text(val, source);
                            if val_text == "false" || val_text == "null" {
                                return CatchBodyKind::TrivialReturn;
                            }
                        }
                    }
                    return CatchBodyKind::ReturnOnly;
                }
                "continue_statement" => return CatchBodyKind::ContinueOnly,
                "expression_statement" => {
                    if let Some(expr) = child.named_child(0) {
                        if expr.kind() == "assignment_expression" {
                            return CatchBodyKind::AssignmentOnly;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    CatchBodyKind::Substantive
}

impl NodePipeline for SilentExceptionPipeline {
    fn name(&self) -> &str {
        "silent_exception"
    }

    fn description(&self) -> &str {
        "Detects catch blocks with empty or trivial bodies"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.catch_query, tree.root_node(), source);

        let catch_type_idx = find_capture_index(&self.catch_query, "catch_type");
        let catch_body_idx = find_capture_index(&self.catch_query, "catch_body");
        let catch_idx = find_capture_index(&self.catch_query, "catch");

        while let Some(m) = matches.next() {
            let type_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == catch_type_idx)
                .map(|c| c.node);
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

            if let (Some(type_node), Some(body_node), Some(catch_node)) =
                (type_node, body_node, catch_node)
            {
                let body_kind = classify_body(body_node, source);

                // Only flag empty or trivial bodies
                if body_kind == CatchBodyKind::Substantive {
                    continue;
                }

                if is_nolint_suppressed(source, catch_node, self.name()) {
                    continue;
                }

                let is_broad = catches_broad_exception(type_node, source);

                // Graduate severity and choose pattern
                let (severity, pattern, message) = match body_kind {
                    CatchBodyKind::Empty => {
                        let sev = if is_broad { "error" } else { "warning" };
                        (sev, "empty_catch", "catch block is empty — log or rethrow the exception")
                    }
                    CatchBodyKind::ReturnOnly => {
                        let sev = if is_broad { "warning" } else { "info" };
                        (sev, "silent_catch", "catch block only returns — consider logging the exception")
                    }
                    CatchBodyKind::TrivialReturn => {
                        (
                            "info",
                            "trivial_catch",
                            "catch block returns a default value — consider logging the exception",
                        )
                    }
                    CatchBodyKind::ContinueOnly => {
                        (
                            "warning",
                            "trivial_catch",
                            "catch block only continues — consider logging the exception",
                        )
                    }
                    CatchBodyKind::AssignmentOnly => {
                        (
                            "info",
                            "trivial_catch",
                            "catch block only sets a variable — consider logging the exception",
                        )
                    }
                    CatchBodyKind::Substantive => unreachable!(),
                };

                let start = catch_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "test.php")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SilentExceptionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_empty_catch_broad_exception() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_catch");
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn detects_return_only_catch() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { return; }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "silent_catch");
    }

    #[test]
    fn clean_catch_with_logging() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { error_log($e->getMessage()); }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_throwable_catch() {
        let src = "<?php\ntry { foo(); } catch (Throwable $e) { }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    // --- New tests ---

    #[test]
    fn detects_specific_exception_empty_catch() {
        // Now ALL empty catches are flagged, not just Exception/Throwable
        let src = "<?php\ntry { foo(); } catch (InvalidArgumentException $e) { }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_catch");
        assert_eq!(findings[0].severity, "warning"); // specific type = warning, not error
    }

    #[test]
    fn detects_continue_only_catch() {
        let src = "<?php\nforeach ($items as $item) { try { process($item); } catch (Exception $e) { continue; } }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_catch");
    }

    #[test]
    fn detects_return_false_catch() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { return false; }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_catch");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn detects_assignment_only_catch() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { $failed = true; }\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_catch");
    }

    #[test]
    fn test_file_suppressed() {
        let src = "<?php\ntry { foo(); } catch (Exception $e) { }\n";
        let findings = parse_and_check_path(src, "tests/FooTest.php");
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "<?php\n// NOLINT(silent_exception)\ntry { foo(); } catch (Exception $e) { }\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
