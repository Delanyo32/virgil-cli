use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

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

    fn is_inside_try_catch(node: tree_sitter::Node) -> bool {
        // Walk up to find if inside a try block's compound_statement
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "compound_statement"
                && let Some(grandparent) = parent.parent()
                    && grandparent.kind() == "try_statement" {
                        return true;
                    }
            // Stop at function boundary
            if parent.kind() == "function_definition" {
                return false;
            }
            current = parent.parent();
        }
        false
    }
}

impl GraphPipeline for ExceptionAcrossBoundaryPipeline {
    fn name(&self) -> &str {
        "exception_across_boundary"
    }

    fn description(&self) -> &str {
        "Detects throw statements inside extern \"C\" blocks — exceptions cannot cross C linkage boundaries"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
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

                if is_nolint_suppressed(source, throw_cap.node, self.name()) {
                    continue;
                }

                // If throw is inside a try-catch, downgrade severity (exception is caught before boundary)
                let severity = if Self::is_inside_try_catch(throw_cap.node) {
                    "warning"
                } else {
                    "error"
                };

                let start = throw_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExceptionAcrossBoundaryPipeline::new().unwrap();
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

    #[test]
    fn throw_in_try_catch_downgraded() {
        let src = r#"
extern "C" {
    void foo() {
        try {
            throw 42;
        } catch(...) {}
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn nolint_suppression() {
        let src = r#"
extern "C" {
    void foo() {
        throw 42; // NOLINT
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
