use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::cpp_primitives::{compile_union_specifier_query, extract_snippet, find_capture_index};

pub struct RawUnionPipeline {
    union_query: Arc<Query>,
}

impl RawUnionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            union_query: compile_union_specifier_query()?,
        })
    }
}

impl Pipeline for RawUnionPipeline {
    fn name(&self) -> &str {
        "raw_union"
    }

    fn description(&self) -> &str {
        "Detects raw union usage — prefer std::variant for type-safe tagged unions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.union_query, tree.root_node(), source);

        let union_def_idx = find_capture_index(&self.union_query, "union_def");

        while let Some(m) = matches.next() {
            let union_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == union_def_idx);

            if let Some(union_cap) = union_cap {
                let union_name_idx = find_capture_index(&self.union_query, "union_name");
                let name = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == union_name_idx)
                    .and_then(|c| c.node.utf8_text(source).ok())
                    .unwrap_or("<anonymous>");

                let start = union_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "raw_union".to_string(),
                    message: format!(
                        "raw `union {name}` — consider using `std::variant` for type safety"
                    ),
                    snippet: extract_snippet(source, union_cap.node, 1),
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RawUnionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_named_union() {
        let src = "union Data { int i; float f; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_union");
        assert!(findings[0].message.contains("Data"));
    }

    #[test]
    fn detects_anonymous_union() {
        let src = "union { int x; float y; } val;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_union");
    }

    #[test]
    fn no_findings_without_union() {
        let src = "struct Foo { int x; float y; };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_unions() {
        let src = r#"
union A { int i; float f; };
union B { char c; double d; };
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }
}
