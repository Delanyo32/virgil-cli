use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;
use crate::language::Language;

const TARGET_METHODS: &[&str] = &[
    "lock", "send", "write", "read", "flush", "recv", "try_lock", "try_send", "try_recv",
];

/// Methods whose ignored result is a logic bug (lock guard dropped immediately).
const ERROR_METHODS: &[&str] = &["lock", "try_lock"];

/// Methods whose ignored result is a warning (I/O or channel operations).
const WARNING_METHODS: &[&str] = &["send", "recv", "try_send", "try_recv", "write", "read", "flush"];

fn severity_for_method(method: &str) -> &'static str {
    if ERROR_METHODS.contains(&method) {
        "error"
    } else if WARNING_METHODS.contains(&method) {
        "warning"
    } else {
        "info"
    }
}

/// Check whether the source line immediately above `row` contains an intentional-ignore marker.
fn has_intentional_comment(source: &[u8], row: usize) -> bool {
    if row == 0 {
        return false;
    }
    let prev_row = row.saturating_sub(1);
    // Split source into lines and look at the line above.
    let text = std::str::from_utf8(source).unwrap_or("");
    if let Some(line) = text.lines().nth(prev_row) {
        let lower = line.to_ascii_lowercase();
        return lower.contains("intentional")
            || lower.contains("ignoring")
            || lower.contains("ok to discard");
    }
    false
}

pub struct MustUseIgnoredPipeline {
    expr_stmt_query: Arc<Query>,
    let_wildcard_query: Arc<Query>,
}

impl MustUseIgnoredPipeline {
    pub fn new() -> Result<Self> {
        let ts_lang = Language::Rust.tree_sitter_language();

        let expr_stmt_str = r#"
(expression_statement
  (call_expression
    function: (field_expression
      field: (field_identifier) @method_name)) @call) @stmt
"#;
        let expr_stmt_query = Query::new(&ts_lang, expr_stmt_str)
            .context("failed to compile expression statement query")?;

        let let_wildcard_str = r#"
(let_declaration
  value: (call_expression
    function: (field_expression
      field: (field_identifier) @method_name)) @call) @let_stmt
"#;
        let let_wildcard_query = Query::new(&ts_lang, let_wildcard_str)
            .context("failed to compile let wildcard query")?;

        Ok(Self {
            expr_stmt_query: Arc::new(expr_stmt_query),
            let_wildcard_query: Arc::new(let_wildcard_query),
        })
    }
}

impl GraphPipeline for MustUseIgnoredPipeline {
    fn name(&self) -> &str {
        "must_use_ignored"
    }

    fn description(&self) -> &str {
        "Detects calls to Result-returning methods whose return value is ignored or explicitly discarded"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        if is_test_file(ctx.file_path) {
            return Vec::new();
        }

        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut findings = Vec::new();

        // Check expression statements (result dropped entirely)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.expr_stmt_query, tree.root_node(), source);

            let name_idx = self
                .expr_stmt_query
                .capture_names()
                .iter()
                .position(|n| *n == "method_name")
                .unwrap();
            let call_idx = self
                .expr_stmt_query
                .capture_names()
                .iter()
                .position(|n| *n == "call")
                .unwrap();

            while let Some(m) = matches.next() {
                let name_node = m.captures.iter().find(|c| c.index as usize == name_idx);
                let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

                if let (Some(name_cap), Some(call_cap)) = (name_node, call_node) {
                    let method = name_cap.node.utf8_text(source).unwrap_or("");
                    if TARGET_METHODS.contains(&method) {
                        let start = call_cap.node.start_position();
                        let snippet = call_cap.node.utf8_text(source).unwrap_or("").to_string();
                        let severity = severity_for_method(method);
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: severity.to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "ignored_result".to_string(),
                            message: format!(
                                "`.{method}()` return value is ignored — this may silently discard an error"
                            ),
                            snippet,
                        });
                    }
                }
            }
        }

        // Check let _ = ... patterns
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.let_wildcard_query, tree.root_node(), source);

            let name_idx = self
                .let_wildcard_query
                .capture_names()
                .iter()
                .position(|n| *n == "method_name")
                .unwrap();
            let call_idx = self
                .let_wildcard_query
                .capture_names()
                .iter()
                .position(|n| *n == "call")
                .unwrap();
            let let_stmt_idx = self
                .let_wildcard_query
                .capture_names()
                .iter()
                .position(|n| *n == "let_stmt")
                .unwrap();

            while let Some(m) = matches.next() {
                let name_node = m.captures.iter().find(|c| c.index as usize == name_idx);
                let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);
                let let_node = m.captures.iter().find(|c| c.index as usize == let_stmt_idx);

                if let (Some(name_cap), Some(call_cap), Some(let_cap)) =
                    (name_node, call_node, let_node)
                {
                    let method = name_cap.node.utf8_text(source).unwrap_or("");

                    // Check if pattern is `_` by examining the let_declaration's pattern field
                    let pat_text = let_cap
                        .node
                        .child_by_field_name("pattern")
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("");

                    if TARGET_METHODS.contains(&method) && pat_text == "_" {
                        let start = call_cap.node.start_position();
                        let snippet = call_cap.node.utf8_text(source).unwrap_or("").to_string();

                        // If line above has intentional-ignore marker, downgrade to info
                        let severity = if has_intentional_comment(source, let_cap.node.start_position().row) {
                            "info"
                        } else {
                            severity_for_method(method)
                        };

                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: severity.to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "discarded_result".to_string(),
                            message: format!(
                                "`let _ = .{method}()` explicitly discards the result — handle the error or use `let _guard = ...`"
                            ),
                            snippet,
                        });
                    }
                }
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MustUseIgnoredPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "test.rs")
    }

    #[test]
    fn detects_ignored_lock() {
        let src = r#"
fn example() {
    mutex.lock();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ignored_result");
        assert!(findings[0].message.contains("lock"));
    }

    #[test]
    fn skips_assigned_lock() {
        let src = r#"
fn example() {
    let guard = mutex.lock();
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_target_method() {
        let src = r#"
fn example() {
    v.push(1);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_discarded_with_wildcard() {
        let src = r#"
fn example() {
    let _ = sender.send(42);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "discarded_result");
    }

    #[test]
    fn skips_named_let_binding() {
        let src = r#"
fn example() {
    let result = sender.send(42);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_file_excluded() {
        let src = r#"fn f() { mutex.lock(); }"#;
        let findings = parse_and_check_path(src, "tests/sync.rs");
        assert!(findings.is_empty());
    }

    #[test]
    fn lock_ignored_is_error() {
        // expression statement: mutex.lock() with result dropped
        let src = r#"
fn f(mutex: &std::sync::Mutex<i32>) {
    mutex.lock();
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty(), "ignored lock should be flagged");
        assert_eq!(
            findings[0].severity, "error",
            "dropped lock guard should be error severity"
        );
    }

    #[test]
    fn flush_ignored_is_warning() {
        let src = r#"
fn f(w: &mut dyn std::io::Write) {
    w.flush();
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.is_empty(), "ignored flush should be flagged");
        // flush maps to "warning" in the new severity scheme
        assert!(
            findings[0].severity == "warning" || findings[0].severity == "info",
            "flush should be warning or info, got {}",
            findings[0].severity
        );
    }

    #[test]
    fn let_underscore_with_intentional_comment_is_info() {
        let src = r#"
fn f(tx: &std::sync::mpsc::Sender<i32>) {
    // intentionally ignoring result
    let _ = tx.send(42);
}
"#;
        let findings = parse_and_check(src);
        // Either not flagged or downgraded to info
        assert!(
            findings.is_empty() || findings[0].severity == "info",
            "intentional discard should be empty or info, got {:?}",
            findings.iter().map(|f| &f.severity).collect::<Vec<_>>()
        );
    }
}
