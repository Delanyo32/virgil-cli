use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::complexity_helpers::count_function_lines;
use crate::audit::primitives::{extract_snippet, find_capture_index};

const QUERY_SRC: &str = r#"
(function_definition
  declarator: (function_declarator
    declarator: (_) @fn_name)
  body: (compound_statement) @fn_body) @func
"#;

fn extract_function_name(name_node: tree_sitter::Node, source: &[u8]) -> String {
    if name_node.kind() == "identifier" {
        return name_node.utf8_text(source).unwrap_or("").to_string();
    }
    let mut cursor = name_node.walk();
    for child in name_node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return child.utf8_text(source).unwrap_or("").to_string();
        }
    }
    name_node
        .utf8_text(source)
        .unwrap_or("<unknown>")
        .to_string()
}

pub struct FunctionLengthPipeline {
    query: Query,
}

impl FunctionLengthPipeline {
    pub fn new() -> Result<Self> {
        let lang = crate::language::Language::C.tree_sitter_language();
        let query = Query::new(&lang, QUERY_SRC)?;
        Ok(Self { query })
    }
}

impl Pipeline for FunctionLengthPipeline {
    fn name(&self) -> &str {
        "function_length"
    }

    fn description(&self) -> &str {
        "Detects C functions that are too long"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.query, "fn_name");
        let body_idx = find_capture_index(&self.query, "fn_body");
        let func_idx = find_capture_index(&self.query, "func");

        while let Some(m) = matches.next() {
            let name_cap = m.captures.iter().find(|c| c.index as usize == name_idx);
            let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);
            let func_cap = m.captures.iter().find(|c| c.index as usize == func_idx);

            if let (Some(name_cap), Some(body_cap), Some(func_cap)) =
                (name_cap, body_cap, func_cap)
            {
                let fn_name = extract_function_name(name_cap.node, source);
                let (lines, statements) = count_function_lines(body_cap.node);
                let start = func_cap.node.start_position();

                if lines > 50 {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "function_too_long".to_string(),
                        message: format!(
                            "function `{fn_name}` is {lines} lines long (threshold: 50)"
                        ),
                        snippet: extract_snippet(source, func_cap.node, 3),
                    });
                }

                if statements > 20 {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "too_many_statements".to_string(),
                        message: format!(
                            "function `{fn_name}` has {statements} statements (threshold: 20)"
                        ),
                        snippet: extract_snippet(source, func_cap.node, 3),
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = FunctionLengthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_long_function() {
        let mut lines = vec!["void long_func() {".to_string()];
        for i in 0..55 {
            lines.push(format!("    int x{i} = {i};"));
        }
        lines.push("}".to_string());
        let src = lines.join("\n");
        let findings = parse_and_check(&src);
        let long_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "function_too_long")
            .collect();
        assert!(!long_findings.is_empty());
    }

    #[test]
    fn clean_short_function() {
        let src = "int short_func(int x) { if (x) { return 1; } return 0; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
