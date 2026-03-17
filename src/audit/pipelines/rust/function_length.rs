use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_function_lines;
use super::primitives;
use crate::audit::primitives::{extract_snippet, find_capture_index};

const LINE_THRESHOLD: usize = 50;
const STATEMENT_THRESHOLD: usize = 20;

pub struct FunctionLengthPipeline {
    query: Arc<Query>,
}

impl FunctionLengthPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            query: primitives::compile_function_item_query()?,
        })
    }
}

impl Pipeline for FunctionLengthPipeline {
    fn name(&self) -> &str {
        "function_length"
    }

    fn description(&self) -> &str {
        "Detects functions that are too long (>50 lines) or have too many statements (>20)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.query, "fn_name");
        let body_idx = find_capture_index(&self.query, "fn_body");
        let def_idx = find_capture_index(&self.query, "fn_def");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == def_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(def_node)) =
                (name_node, body_node, def_node)
            {
                let name = name_node.utf8_text(source).unwrap_or("<unknown>");
                let (lines, stmts) = count_function_lines(body_node);

                if lines > LINE_THRESHOLD {
                    let start = def_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "function_length".to_string(),
                        pattern: "function_too_long".to_string(),
                        message: format!(
                            "Function '{name}' is {lines} lines long (threshold: {LINE_THRESHOLD})"
                        ),
                        snippet: extract_snippet(source, def_node, 3),
                    });
                }

                if stmts > STATEMENT_THRESHOLD {
                    let start = def_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: "function_length".to_string(),
                        pattern: "too_many_statements".to_string(),
                        message: format!(
                            "Function '{name}' has {stmts} statements (threshold: {STATEMENT_THRESHOLD})"
                        ),
                        snippet: extract_snippet(source, def_node, 3),
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
        let pipeline = FunctionLengthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_long_function() {
        // Generate a function with 55+ lines
        let mut body_lines = String::new();
        for i in 0..55 {
            body_lines.push_str(&format!("    let x{i} = {i};\n"));
        }
        let src = format!("fn long_function() {{\n{body_lines}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "function_too_long"));
    }

    #[test]
    fn detects_too_many_statements() {
        let mut stmts = String::new();
        for i in 0..22 {
            stmts.push_str(&format!("    let x{i} = {i};\n"));
        }
        let src = format!("fn many_stmts() {{\n{stmts}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "too_many_statements"));
    }

    #[test]
    fn clean_short_function() {
        let src = r#"
fn simple() {
    let x = 1;
    let y = 2;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
