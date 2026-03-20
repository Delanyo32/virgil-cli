use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_throw_statement_query, extract_snippet, find_capture_index, node_text,
};

pub struct ExceptionAcrossBoundaryPipeline {
    throw_query: Arc<Query>,
}

impl ExceptionAcrossBoundaryPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            throw_query: compile_throw_statement_query()?,
        })
    }

    fn is_inside_extern_c(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "linkage_specification" {
                // Check if the linkage string is "C"
                let mut cursor = parent.walk();
                for child in parent.children(&mut cursor) {
                    if child.kind() == "string_literal" {
                        let text = node_text(child, source);
                        if text == "\"C\"" {
                            return true;
                        }
                    }
                }
            }
            current = parent.parent();
        }
        false
    }
}

impl Pipeline for ExceptionAcrossBoundaryPipeline {
    fn name(&self) -> &str {
        "exception_across_boundary"
    }

    fn description(&self) -> &str {
        "Detects throw statements inside extern \"C\" blocks — exceptions cannot cross C linkage boundaries"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.throw_query, tree.root_node(), source);

        let throw_idx = find_capture_index(&self.throw_query, "throw_stmt");

        while let Some(m) = matches.next() {
            let throw_cap = m.captures.iter().find(|c| c.index as usize == throw_idx);

            if let Some(throw_cap) = throw_cap {
                if !Self::is_inside_extern_c(throw_cap.node, source) {
                    continue;
                }

                let start = throw_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "error".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "exception_across_boundary".to_string(),
                    message: "throwing inside `extern \"C\"` — exceptions cannot propagate across C linkage boundaries (undefined behavior)".to_string(),
                    snippet: extract_snippet(source, throw_cap.node, 1),
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
        let pipeline = ExceptionAcrossBoundaryPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_throw_in_extern_c() {
        let src = r#"
extern "C" {
    void foo() {
        throw 42;
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "exception_across_boundary");
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn no_finding_for_throw_in_cpp() {
        let src = r#"
void foo() {
    throw std::runtime_error("oops");
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_for_extern_c_without_throw() {
        let src = r#"
extern "C" {
    void foo() {
        int x = 42;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_nested_throw_in_extern_c() {
        let src = r#"
extern "C" {
    void foo() {
        if (true) {
            throw -1;
        }
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
extern "C" {
    void bar() { throw 0; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings[0].pipeline, "exception_across_boundary");
        assert_eq!(findings[0].severity, "error");
    }
}
