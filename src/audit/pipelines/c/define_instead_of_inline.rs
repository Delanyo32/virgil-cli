use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_preproc_function_def_query, extract_snippet, find_capture_index};

pub struct DefineInsteadOfInlinePipeline {
    macro_query: Arc<Query>,
}

impl DefineInsteadOfInlinePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            macro_query: compile_preproc_function_def_query()?,
        })
    }
}

impl Pipeline for DefineInsteadOfInlinePipeline {
    fn name(&self) -> &str {
        "define_instead_of_inline"
    }

    fn description(&self) -> &str {
        "Detects function-like #define macros that could be inline functions for better type safety"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.macro_query, tree.root_node(), source);

        let macro_name_idx = find_capture_index(&self.macro_query, "macro_name");
        let macro_def_idx = find_capture_index(&self.macro_query, "macro_def");

        while let Some(m) = matches.next() {
            let name_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == macro_name_idx);
            let def_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == macro_def_idx);

            if let (Some(name_cap), Some(def_cap)) = (name_cap, def_cap) {
                let macro_name = name_cap.node.utf8_text(source).unwrap_or("");
                let start = def_cap.node.start_position();

                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "function_like_macro".to_string(),
                    message: format!(
                        "function-like macro `{macro_name}` — consider using an inline function for type safety"
                    ),
                    snippet: extract_snippet(source, def_cap.node, 1),
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
        let pipeline = DefineInsteadOfInlinePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_function_like_macro() {
        let src = "#define ADD(a, b) ((a) + (b))";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "function_like_macro");
        assert!(findings[0].message.contains("ADD"));
    }

    #[test]
    fn skips_value_macro() {
        let src = "#define MAX 100";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
