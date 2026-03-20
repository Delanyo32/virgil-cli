use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{self, extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

const PATH_CHECK_METHODS: &[&str] = &["exists", "is_file", "is_dir"];
const FILE_OP_METHODS: &[&str] = &["open", "read", "write", "create", "remove"];

pub struct ToctouPipeline {
    if_query: Arc<Query>,
}

impl ToctouPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            if_query: primitives::compile_if_expression_query()?,
        })
    }

    fn text_contains_path_check(text: &str) -> bool {
        PATH_CHECK_METHODS
            .iter()
            .any(|method| text.contains(&format!(".{method}(")))
    }

    fn text_contains_file_op(text: &str) -> bool {
        FILE_OP_METHODS.iter().any(|method| {
            text.contains(&format!(".{method}(")) || text.contains(&format!("::{method}("))
        })
    }
}

impl Pipeline for ToctouPipeline {
    fn name(&self) -> &str {
        "toctou"
    }

    fn description(&self) -> &str {
        "Detects time-of-check-time-of-use races: path existence checks followed by file operations"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.if_query, tree.root_node(), source);

        let condition_idx = find_capture_index(&self.if_query, "if_condition");
        let body_idx = find_capture_index(&self.if_query, "if_body");
        let if_expr_idx = find_capture_index(&self.if_query, "if_expr");

        while let Some(m) = matches.next() {
            let condition_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == condition_idx)
                .map(|c| c.node);
            let body_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let if_expr_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == if_expr_idx)
                .map(|c| c.node);

            if let (Some(cond_n), Some(body_n), Some(if_n)) = (condition_cap, body_cap, if_expr_cap)
            {
                let cond_text = node_text(cond_n, source);
                let body_text = node_text(body_n, source);

                if Self::text_contains_path_check(cond_text)
                    && Self::text_contains_file_op(body_text)
                {
                    let start = if_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "path_check_use_race".to_string(),
                        message:
                            "TOCTOU: path check followed by file operation creates a race window"
                                .to_string(),
                        snippet: extract_snippet(source, if_n, 3),
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ToctouPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_path_check_use() {
        let src = r#"
fn f() {
    let p = std::path::Path::new("x");
    if p.exists() {
        std::fs::File::open(p);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "path_check_use_race");
    }

    #[test]
    fn ignores_if_without_path_check() {
        let src = r#"
fn f() {
    if true {
        std::fs::File::open("x");
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_path_check_without_file_op() {
        let src = r#"
fn f() {
    let p = std::path::Path::new("x");
    if p.exists() {
        println!("yes");
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
