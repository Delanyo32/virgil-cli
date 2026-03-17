use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_function_lines;
use crate::audit::primitives::{extract_snippet, find_capture_index, node_text};

const QUERY_SRC: &str = r#"
[
  (function_definition
    name: (name) @fn_name
    body: (compound_statement) @fn_body) @func
  (method_declaration
    name: (name) @fn_name
    body: (compound_statement) @fn_body) @func
]
"#;

pub struct FunctionLengthPipeline {
    query: Query,
}

impl FunctionLengthPipeline {
    pub fn new() -> Result<Self> {
        let lang = crate::language::Language::Php.tree_sitter_language();
        let query = Query::new(&lang, QUERY_SRC)?;
        Ok(Self { query })
    }
}

impl Pipeline for FunctionLengthPipeline {
    fn name(&self) -> &str {
        "function_length"
    }

    fn description(&self) -> &str {
        "Detects PHP functions and methods that are too long"
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
                let fn_name = node_text(name_cap.node, source);
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = FunctionLengthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_long_function() {
        let mut lines = vec!["<?php".to_string(), "function long_func() {".to_string()];
        for i in 0..55 {
            lines.push(format!("    $x{i} = {i};"));
        }
        lines.push("}".to_string());
        lines.push("?>".to_string());
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
        let src = "<?php\nfunction short() { $x = 1; return $x; }\n?>";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
