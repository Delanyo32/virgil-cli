use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

const TARGET_METHODS: &[&str] = &[
    "lock", "send", "write", "read", "flush", "recv", "try_lock", "try_send", "try_recv",
];

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

impl Pipeline for MustUseIgnoredPipeline {
    fn name(&self) -> &str {
        "must_use_ignored"
    }

    fn description(&self) -> &str {
        "Detects calls to Result-returning methods whose return value is ignored or explicitly discarded"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
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
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MustUseIgnoredPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
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
}
