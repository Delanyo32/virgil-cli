use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_qualified_identifier_query, extract_snippet, find_capture_index,
    has_using_namespace_std, is_inside_loop, node_text,
};

use crate::language::Language;

pub struct EndlFlushPipeline {
    qualified_id_query: Arc<Query>,
    identifier_query: Arc<Query>,
}

impl EndlFlushPipeline {
    pub fn new() -> Result<Self> {
        let lang = Language::Cpp.tree_sitter_language();
        let id_query = Query::new(&lang, "(identifier) @id")
            .map_err(|e| anyhow::anyhow!("failed to compile identifier query for C++: {e}"))?;
        Ok(Self {
            qualified_id_query: compile_qualified_identifier_query()?,
            identifier_query: Arc::new(id_query),
        })
    }

    fn is_cerr_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        // Check if this endl/flush is used in a << expression with cerr/std::cerr on the left
        if let Some(parent) = node.parent()
            && parent.kind() == "binary_expression" {
                if let Some(left) = parent.child_by_field_name("left") {
                    let left_text = node_text(left, source);
                    if left_text.contains("cerr") {
                        return true;
                    }
                }
                // Walk up chained << expressions
                if let Some(grandparent) = parent.parent()
                    && grandparent.kind() == "binary_expression" {
                        return Self::is_cerr_context(parent, source);
                    }
            }
        false
    }
}

impl GraphPipeline for EndlFlushPipeline {
    fn name(&self) -> &str {
        "endl_flush"
    }

    fn description(&self) -> &str {
        "Detects std::endl usage — prefer '\\n' to avoid unnecessary stream flushing"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();

        // Check for qualified std::endl and std::flush
        {
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
                    if text != "std::endl" && text != "std::flush" {
                        continue;
                    }

                    if is_nolint_suppressed(source, id_cap.node, self.name()) {
                        continue;
                    }

                    // Skip if used with cerr (unbuffered by default)
                    if Self::is_cerr_context(id_cap.node, source) {
                        continue;
                    }

                    let severity = if is_inside_loop(id_cap.node) {
                        "warning"
                    } else {
                        "info"
                    };

                    let start = id_cap.node.start_position();
                    let message = if text == "std::flush" {
                        "`std::flush` explicitly flushes the stream — consider whether flushing is needed".to_string()
                    } else {
                        "`std::endl` flushes the stream — use `'\\n'` unless an explicit flush is needed".to_string()
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "endl_flush".to_string(),
                        message,
                        snippet: extract_snippet(source, id_cap.node, 1),
                    });
                }
            }
        }

        // Check for bare endl/flush after `using namespace std`
        if has_using_namespace_std(tree.root_node(), source) {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.identifier_query, tree.root_node(), source);
            let id_idx = find_capture_index(&self.identifier_query, "id");

            while let Some(m) = matches.next() {
                let id_cap = m.captures.iter().find(|c| c.index as usize == id_idx);

                if let Some(id_cap) = id_cap {
                    let text = node_text(id_cap.node, source);
                    if text != "endl" && text != "flush" {
                        continue;
                    }

                    // Skip if this is part of a qualified identifier (already handled above)
                    if let Some(parent) = id_cap.node.parent()
                        && parent.kind() == "qualified_identifier" {
                            continue;
                        }

                    if is_nolint_suppressed(source, id_cap.node, self.name()) {
                        continue;
                    }

                    if Self::is_cerr_context(id_cap.node, source) {
                        continue;
                    }

                    let severity = if is_inside_loop(id_cap.node) {
                        "warning"
                    } else {
                        "info"
                    };

                    let start = id_cap.node.start_position();
                    let message = if text == "flush" {
                        "`flush` explicitly flushes the stream — consider whether flushing is needed".to_string()
                    } else {
                        "`endl` flushes the stream — use `'\\n'` unless an explicit flush is needed".to_string()
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "endl_flush".to_string(),
                        message,
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = EndlFlushPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.cpp",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn detects_std_flush() {
        let src = "void f() { std::cout << std::flush; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("flush"));
    }

    #[test]
    fn endl_in_loop_warning() {
        let src = r#"
void f() {
    for (int i = 0; i < 100; i++) {
        std::cout << i << std::endl;
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn nolint_suppression() {
        let src = "void f() { std::cout << std::endl; // NOLINT }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
