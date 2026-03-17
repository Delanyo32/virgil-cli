use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::c_primitives::{
    compile_expression_statement_call_query, extract_snippet, find_capture_index,
};

const DANGEROUS_FUNCTIONS: &[&str] = &[
    "fwrite", "fread", "fclose", "fopen", "fgets", "fputs", "strncpy", "snprintf", "memcpy",
    "memmove", "read", "write", "close", "open",
];

pub struct IgnoredReturnValuesPipeline {
    expr_call_query: Arc<Query>,
}

impl IgnoredReturnValuesPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            expr_call_query: compile_expression_statement_call_query()?,
        })
    }
}

impl Pipeline for IgnoredReturnValuesPipeline {
    fn name(&self) -> &str {
        "ignored_return_values"
    }

    fn description(&self) -> &str {
        "Detects discarded return values from functions whose return value indicates success/failure"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.expr_call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.expr_call_query, "fn_name");
        let call_idx = find_capture_index(&self.expr_call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(fn_cap), Some(call_cap)) = (fn_cap, call_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                if !DANGEROUS_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                let start = call_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "ignored_return_value".to_string(),
                    message: format!(
                        "return value of `{fn_name}()` is discarded — check for errors"
                    ),
                    snippet: extract_snippet(source, call_cap.node, 1),
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = IgnoredReturnValuesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_ignored_fwrite() {
        let src = "void f() { fwrite(buf, 1, n, fp); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "ignored_return_value");
        assert!(findings[0].message.contains("fwrite"));
    }

    #[test]
    fn skips_assigned_return() {
        let src = "void f() { size_t n = fwrite(buf, 1, sz, fp); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_ignored_fclose() {
        let src = "void f() { fclose(fp); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("fclose"));
    }
}
