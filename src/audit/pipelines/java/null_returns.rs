use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::java_primitives::{
    compile_return_null_query, extract_snippet, find_capture_index, node_text,
};

pub struct NullReturnsPipeline {
    return_null_query: Arc<Query>,
}

impl NullReturnsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            return_null_query: compile_return_null_query()?,
        })
    }
}

impl Pipeline for NullReturnsPipeline {
    fn name(&self) -> &str {
        "null_returns"
    }

    fn description(&self) -> &str {
        "Detects methods that return null — consider returning Optional<T> instead"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.return_null_query, tree.root_node(), source);

        let return_stmt_idx = find_capture_index(&self.return_null_query, "return_stmt");

        while let Some(m) = matches.next() {
            let return_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == return_stmt_idx)
                .map(|c| c.node);

            let Some(return_node) = return_node else {
                continue;
            };

            // Walk parent chain to find enclosing method_declaration
            let mut parent = return_node.parent();
            let mut method_name = None;
            while let Some(p) = parent {
                match p.kind() {
                    "method_declaration" => {
                        method_name = p
                            .child_by_field_name("name")
                            .map(|n| node_text(n, source).to_string());
                        break;
                    }
                    "constructor_declaration" => {
                        // Skip constructors
                        break;
                    }
                    _ => {
                        parent = p.parent();
                    }
                }
            }

            let Some(method_name) = method_name else {
                continue;
            };

            let start = return_node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "null_return".to_string(),
                message: format!(
                    "method `{method_name}` returns null — consider returning Optional<T>"
                ),
                snippet: extract_snippet(source, return_node, 3),
            });
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
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = NullReturnsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_null_return() {
        let src = "class Foo { String findUser() { return null; } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "null_return");
        assert!(findings[0].message.contains("findUser"));
    }

    #[test]
    fn skips_constructor() {
        let src = "class Foo { Foo() { return null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_non_null_return() {
        let src = "class Foo { String getName() { return \"hello\"; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
