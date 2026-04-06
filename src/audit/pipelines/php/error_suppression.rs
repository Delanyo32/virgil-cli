use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_error_suppression_query, extract_snippet, find_capture_index, node_text,
};

/// Functions where @ suppression is a recognized PHP idiom (non-critical filesystem ops).
const SAFE_SUPPRESSION_TARGETS: &[&str] = &[
    "unlink",
    "session_start",
    "mkdir",
    "rmdir",
    "fopen",
    "fclose",
    "file_get_contents",
    "file_put_contents",
    "ini_set",
    "chmod",
    "chown",
    "rename",
];

pub struct ErrorSuppressionPipeline {
    suppress_query: Arc<Query>,
}

impl ErrorSuppressionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            suppress_query: compile_error_suppression_query()?,
        })
    }

    /// Extract the function name from the suppressed expression, if it's a function call.
    fn suppressed_function_name<'a>(
        suppress_node: tree_sitter::Node<'a>,
        source: &'a [u8],
    ) -> Option<&'a str> {
        // The child of error_suppression_expression is the suppressed expression
        let child = suppress_node.named_child(0)?;
        if child.kind() == "function_call_expression" {
            let func = child.child_by_field_name("function")?;
            if func.kind() == "name" || func.kind() == "qualified_name" {
                return Some(node_text(func, source));
            }
        }
        None
    }

    /// Check if any ancestor is a try_statement (the @ is inside a try block).
    fn is_inside_try(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "try_statement" {
                return true;
            }
            current = parent.parent();
        }
        false
    }
}

impl NodePipeline for ErrorSuppressionPipeline {
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
            let cap = m.captures.iter().find(|c| c.index as usize == suppress_idx);

            if let Some(cap) = cap {
                let node = cap.node;

                if is_nolint_suppressed(source, node, self.name()) {
                    continue;
                }

                // Suppress @ inside try/catch -- error handling already present
                if Self::is_inside_try(node) {
                    continue;
                }

                let fn_name = Self::suppressed_function_name(node, source);

                // Downgrade safe idioms to info
                let severity = if let Some(name) = fn_name {
                    if SAFE_SUPPRESSION_TARGETS.contains(&name) {
                        "info"
                    } else {
                        "warning"
                    }
                } else {
                    "warning"
                };

                let message = if let Some(name) = fn_name {
                    format!(
                        "@ suppresses errors from `{name}()` — use proper error handling instead"
                    )
                } else {
                    "error suppression operator @ hides failures — use proper error handling"
                        .to_string()
                };

                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "at_operator".to_string(),
                    message,
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

    // --- New tests ---

    #[test]
    fn at_unlink_safe_idiom() {
        let src = "<?php\n@unlink('/tmp/old.txt');\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
        assert!(findings[0].message.contains("unlink"));
    }

    #[test]
    fn at_session_start_safe_idiom() {
        let src = "<?php\n@session_start();\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn at_inside_try_catch_suppressed() {
        let src = "<?php\ntry { @file_get_contents('x'); } catch (Exception $e) {}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn at_unknown_function_warning() {
        let src = "<?php\n@some_risky_operation();\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "<?php\n// NOLINT(error_suppression)\n@file_get_contents('x');\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
