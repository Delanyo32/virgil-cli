use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_function_lines;
use crate::language::Language;

const LINE_THRESHOLD: usize = 50;
const STATEMENT_THRESHOLD: usize = 20;

const FUNCTION_QUERY: &str = r#"
(function_definition
  declarator: (function_declarator
    declarator: (_) @fn_name)
  body: (compound_statement) @fn_body) @func
"#;

fn extract_function_name(name_node: tree_sitter::Node, source: &[u8]) -> String {
    if name_node.kind() == "identifier" {
        return name_node.utf8_text(source).unwrap_or("").to_string();
    }
    if name_node.kind() == "qualified_identifier" || name_node.kind() == "field_identifier" {
        return name_node.utf8_text(source).unwrap_or("").to_string();
    }
    let mut cursor = name_node.walk();
    for child in name_node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "field_identifier" {
            return child.utf8_text(source).unwrap_or("").to_string();
        }
    }
    name_node
        .utf8_text(source)
        .unwrap_or("<unknown>")
        .to_string()
}

pub struct FunctionLengthPipeline {
    query: Arc<Query>,
}

impl FunctionLengthPipeline {
    pub fn new() -> Result<Self> {
        let ts_lang = Language::Cpp.tree_sitter_language();
        let query = Query::new(&ts_lang, FUNCTION_QUERY)?;
        Ok(Self {
            query: Arc::new(query),
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
        let func_idx = find_capture_index(&self.query, "func");

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
            let func_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == func_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(func_node)) =
                (name_node, body_node, func_node)
            {
                let name = extract_function_name(name_node, source);
                let (lines, stmts) = count_function_lines(body_node);
                let start = func_node.start_position();

                if lines > LINE_THRESHOLD {
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
                        snippet: extract_snippet(source, func_node, 3),
                    });
                }

                if stmts > STATEMENT_THRESHOLD {
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
                        snippet: extract_snippet(source, func_node, 3),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = FunctionLengthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_long_function() {
        let mut body_lines = String::new();
        for i in 0..52 {
            body_lines.push_str(&format!("    int x{i} = {i};\n"));
        }
        let src = format!("void longFunc() {{\n{body_lines}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "function_too_long"));
    }

    #[test]
    fn detects_too_many_statements() {
        // Use expression_statement nodes which count_statements recognizes
        let mut stmts = String::new();
        stmts.push_str("    int x = 0;\n");
        for i in 0..22 {
            stmts.push_str(&format!("    x = {i};\n"));
        }
        let src = format!("void manyStmts(int x) {{\n{stmts}}}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "too_many_statements"));
    }

    #[test]
    fn clean_short_function() {
        let src = r#"
void simple() {
    int x = 1;
    int y = 2;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
