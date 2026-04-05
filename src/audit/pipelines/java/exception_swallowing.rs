use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::has_suppress_warnings;

use super::primitives::{
    compile_catch_clause_query, extract_snippet, find_capture_index, node_text,
};

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

impl GraphPipeline for ExceptionSwallowingPipeline {
    fn name(&self) -> &str {
        "exception_swallowing"
    }

    fn description(&self) -> &str {
        "Detects catch blocks that swallow exceptions (empty, printStackTrace-only, return null, System.out/err.println, or getMessage no-op)"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

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
                // Skip if @SuppressWarnings("empty-catch") is present
                if has_suppress_warnings(catch_node, source, "empty-catch") {
                    continue;
                }

                // Skip if catch block contains a logging call
                if catch_body_has_logging(body_node, source) {
                    continue;
                }

                let exception_type = catch_exception_type(catch_node, source);
                let broad = is_broad_exception(&exception_type);
                let named_count = body_node.named_child_count();

                if named_count == 0 {
                    // Check for comment-only catch blocks with deliberate intent
                    if has_deliberate_comment(body_node, source) {
                        continue;
                    }

                    let severity = if broad { "error" } else { "warning" };

                    let start = catch_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "empty_catch".to_string(),
                        message: "empty catch block silently swallows exception \u{2014} log or rethrow instead".to_string(),
                        snippet: extract_snippet(source, catch_node, 3),
                    });
                } else if named_count == 1 {
                    let child = body_node.named_child(0).unwrap();
                    let finding = if is_printstacktrace(child, source) {
                        let severity = if broad { "error" } else { "warning" };
                        Some((
                            "printstacktrace_only",
                            "catch block only calls printStackTrace() \u{2014} use proper logging instead",
                            severity,
                        ))
                    } else if is_return_null(child) {
                        let severity = if broad { "error" } else { "info" };
                        Some((
                            "catch_return_null",
                            "catch block returns null \u{2014} propagate the exception or return Optional",
                            severity,
                        ))
                    } else if is_system_print(child, source) {
                        let severity = if broad { "error" } else { "warning" };
                        Some((
                            "system_print_in_catch",
                            "catch block uses System.out/err.println \u{2014} use proper logging instead",
                            severity,
                        ))
                    } else if is_get_message_noop(child, source) {
                        let severity = if broad { "error" } else { "warning" };
                        Some((
                            "getmessage_noop",
                            "catch block calls getMessage() without using the result \u{2014} log or rethrow instead",
                            severity,
                        ))
                    } else {
                        None
                    };

                    if let Some((pattern, message, severity)) = finding {
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
                } else {
                    // named_count >= 2: check if ALL statements are weak handling
                    let mut all_weak = true;
                    let mut has_printstacktrace = false;
                    let mut has_return_null = false;
                    let mut has_system_print = false;
                    let mut has_getmessage = false;

                    for i in 0..named_count {
                        let child = body_node.named_child(i).unwrap();
                        if is_printstacktrace(child, source) {
                            has_printstacktrace = true;
                        } else if is_return_null(child) {
                            has_return_null = true;
                        } else if is_system_print(child, source) {
                            has_system_print = true;
                        } else if is_get_message_noop(child, source) {
                            has_getmessage = true;
                        } else {
                            all_weak = false;
                            break;
                        }
                    }

                    if all_weak {
                        let (pattern, message) = if has_printstacktrace && has_return_null {
                            (
                                "catch_swallow_combo",
                                "catch block calls printStackTrace() then returns null \u{2014} use proper logging and error handling",
                            )
                        } else if has_system_print && has_return_null {
                            (
                                "catch_swallow_combo",
                                "catch block prints to System.out/err and returns null \u{2014} use proper logging and error handling",
                            )
                        } else if has_getmessage && has_return_null {
                            (
                                "catch_swallow_combo",
                                "catch block calls getMessage() and returns null \u{2014} use proper logging and error handling",
                            )
                        } else {
                            (
                                "catch_swallow_combo",
                                "catch block contains only weak exception handling \u{2014} use proper logging and error handling",
                            )
                        };

                        let severity = if broad { "error" } else { "warning" };

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
            }
        }

        findings
    }
}

/// Extract the exception type from a catch clause node.
fn catch_exception_type(catch_node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = catch_node.walk();
    for child in catch_node.children(&mut cursor) {
        if child.kind() == "catch_formal_parameter" {
            let mut inner = child.walk();
            for c in child.children(&mut inner) {
                if c.kind() == "catch_type" || c.kind() == "type_identifier" {
                    return node_text(c, source).to_string();
                }
            }
        }
    }
    "Unknown".to_string()
}

/// Check if the exception type is a broad catch (Exception or Throwable).
fn is_broad_exception(type_text: &str) -> bool {
    type_text == "Exception"
        || type_text == "Throwable"
        || type_text.ends_with(".Exception")
        || type_text.ends_with(".Throwable")
}

/// Check if a catch block body contains only comments with deliberate-intent keywords.
fn has_deliberate_comment(body_node: tree_sitter::Node, source: &[u8]) -> bool {
    let child_count = body_node.child_count();
    let mut has_comment = false;

    for i in 0..child_count {
        let child = body_node.child(i).unwrap();
        let kind = child.kind();
        // Skip braces and whitespace
        if kind == "{" || kind == "}" {
            continue;
        }
        if kind == "line_comment" || kind == "block_comment" {
            has_comment = true;
            let text = child
                .utf8_text(source)
                .unwrap_or("")
                .to_lowercase();
            let deliberate_keywords = ["expected", "ignore", "intentionally", "deliberate", "noop"];
            if deliberate_keywords.iter().any(|kw| text.contains(kw)) {
                continue;
            }
            // Comment without deliberate keyword — not intentional
            return false;
        }
        // Any non-comment, non-brace child means it's not comment-only
        return false;
    }

    has_comment
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
        if let Some(child) = node.named_child(i)
            && child.kind() == "null_literal"
        {
            return true;
        }
    }
    false
}

/// Check if a node is a System.out.println or System.err.println call.
fn is_system_print(node: tree_sitter::Node, source: &[u8]) -> bool {
    if node.kind() != "expression_statement" {
        return false;
    }
    let Some(expr) = node.named_child(0) else {
        return false;
    };
    if expr.kind() != "method_invocation" {
        return false;
    }
    let Some(obj) = expr.child_by_field_name("object") else {
        return false;
    };
    let obj_text = obj.utf8_text(source).unwrap_or("");
    obj_text == "System.out" || obj_text == "System.err"
}

/// Check if a node is a standalone e.getMessage() call (no-op — result is unused).
fn is_get_message_noop(node: tree_sitter::Node, source: &[u8]) -> bool {
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
    name.utf8_text(source).unwrap_or("") == "getMessage"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExceptionSwallowingPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Test.java",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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
        let src = "class Foo { void m() { try { } catch (Exception e) { e.printStackTrace(); } } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "printstacktrace_only");
    }

    #[test]
    fn detects_return_null_catch() {
        let src = "class Foo { Object m() { try { return new Object(); } catch (Exception e) { return null; } } }";
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

    #[test]
    fn test_two_statement_catch() {
        let src = r#"class Foo { void m() { try { } catch (Exception e) { e.printStackTrace(); return null; } } }"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].pattern, "catch_swallow_combo");
    }

    #[test]
    fn test_system_out_println() {
        let src = r#"class Foo { void m() { try { } catch (Exception e) { System.out.println(e); } } }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "system_print_in_catch");
    }

    #[test]
    fn test_broad_vs_specific_exception() {
        let src_broad = r#"class Foo { void m() { try { } catch (Throwable t) { } } }"#;
        let src_specific = r#"class Foo { void m() { try { } catch (FileNotFoundException e) { } } }"#;
        let broad = parse_and_check(src_broad);
        let specific = parse_and_check(src_specific);
        assert_eq!(broad[0].severity, "error");
        assert_eq!(specific[0].severity, "warning");
    }

    #[test]
    fn test_catch_get_message_noop() {
        let src = r#"class Foo { void m() { try { } catch (Exception e) { e.getMessage(); } } }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "getmessage_noop");
    }

    #[test]
    fn test_suppress_warnings() {
        let src = r#"class Foo { @SuppressWarnings("empty-catch") void m() { try { } catch (Exception e) { } } }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
