use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::cpp_primitives::{compile_qualified_identifier_query, extract_snippet, find_capture_index, node_text};

pub struct EndlFlushPipeline {
    qualified_id_query: Arc<Query>,
}

impl EndlFlushPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            qualified_id_query: compile_qualified_identifier_query()?,
        })
    }
}

impl Pipeline for EndlFlushPipeline {
    fn name(&self) -> &str {
        "endl_flush"
    }

    fn description(&self) -> &str {
        "Detects std::endl usage — prefer '\\n' to avoid unnecessary stream flushing"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.qualified_id_query, tree.root_node(), source);

        let qualified_id_idx = find_capture_index(&self.qualified_id_query, "qualified_id");

        while let Some(m) = matches.next() {
            let id_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == qualified_id_idx);

            if let Some(id_cap) = id_cap {
                let text = node_text(id_cap.node, source);
                if text == "std::endl" {
                    let start = id_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "endl_flush".to_string(),
                        message: "`std::endl` flushes the stream — use `'\\n'` unless an explicit flush is needed".to_string(),
                        snippet: extract_snippet(source, id_cap.node, 1),
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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = EndlFlushPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_std_endl() {
        let src = r#"
#include <iostream>
void f() { std::cout << "hello" << std::endl; }
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "endl_flush");
        assert!(findings[0].message.contains("std::endl"));
    }

    #[test]
    fn no_finding_for_newline_char() {
        let src = r#"
#include <iostream>
void f() { std::cout << "hello\n"; }
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_endl() {
        let src = r#"
#include <iostream>
void f() {
    std::cout << "a" << std::endl;
    std::cout << "b" << std::endl;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn no_finding_for_other_qualified_id() {
        let src = "void f() { std::string s; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"void f() { std::cout << std::endl; }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
        assert_eq!(findings[0].pipeline, "endl_flush");
    }
}
