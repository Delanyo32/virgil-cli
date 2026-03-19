use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_generic_type_query, extract_snippet, find_capture_index, node_text,
};

pub struct RecordStringAnyPipeline {
    query: Arc<Query>,
    name_idx: usize,
    args_idx: usize,
}

impl RecordStringAnyPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_generic_type_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        let args_idx = find_capture_index(&query, "args");
        Ok(Self {
            query,
            name_idx,
            args_idx,
        })
    }
}

impl Pipeline for RecordStringAnyPipeline {
    fn name(&self) -> &str {
        "record_string_any"
    }

    fn description(&self) -> &str {
        "Detects `Record<string, any>` which is a type-unsafe catch-all"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let type_name = m
                .captures
                .iter()
                .find(|c| c.index as usize == self.name_idx)
                .map(|c| node_text(c.node, source))
                .unwrap_or("");

            if type_name != "Record" {
                continue;
            }

            let args_node = match m
                .captures
                .iter()
                .find(|c| c.index as usize == self.args_idx)
            {
                Some(c) => c.node,
                None => continue,
            };

            // Check if any type argument is `any`
            let mut has_any = false;
            let mut args_cursor = args_node.walk();
            for child in args_node.named_children(&mut args_cursor) {
                if child.kind() == "predefined_type" && node_text(child, source) == "any" {
                    has_any = true;
                    break;
                }
            }

            if has_any {
                let generic_node = m.captures.first().map(|c| c.node).unwrap_or(args_node);
                let start = generic_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "record_any".to_string(),
                    message: "`Record<string, any>` is a type-unsafe catch-all — define a specific value type or use `unknown`".to_string(),
                    snippet: extract_snippet(source, generic_node, 1),
                });
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
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RecordStringAnyPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_record_string_any() {
        let findings = parse_and_check("let x: Record<string, any> = {};");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "record_any");
    }

    #[test]
    fn skips_record_string_number() {
        let findings = parse_and_check("let x: Record<string, number> = {};");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_record_generic() {
        let findings = parse_and_check("let x: Map<string, any>;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_record_string_unknown() {
        let findings = parse_and_check("let x: Record<string, unknown> = {};");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        let pipeline = RecordStringAnyPipeline::new(Language::Tsx).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Tsx.tree_sitter_language())
            .unwrap();
        let tree = parser
            .parse("let x: Record<string, any> = {};", None)
            .unwrap();
        let findings = pipeline.check(&tree, b"let x: Record<string, any> = {};", "test.tsx");
        assert_eq!(findings.len(), 1);
    }
}
