use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::go_primitives::{compile_selector_call_query, extract_snippet, find_capture_index, node_text};

pub struct ContextNotPropagatedPipeline {
    call_query: Arc<Query>,
}

impl ContextNotPropagatedPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_selector_call_query()?,
        })
    }

    fn enclosing_function_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_declaration" {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    return Some(node_text(name_node, source).to_string());
                }
            }
            current = parent.parent();
        }
        None
    }
}

impl Pipeline for ContextNotPropagatedPipeline {
    fn name(&self) -> &str {
        "context_not_propagated"
    }

    fn description(&self) -> &str {
        "Detects context.Background()/context.TODO() in non-main/non-init functions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let pkg_idx = find_capture_index(&self.call_query, "pkg");
        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let pkg_node = m.captures.iter().find(|c| c.index as usize == pkg_idx).map(|c| c.node);
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx).map(|c| c.node);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx).map(|c| c.node);

            if let (Some(pkg_node), Some(method_node), Some(call_node)) = (pkg_node, method_node, call_node) {
                let pkg_name = node_text(pkg_node, source);
                let method_name = node_text(method_node, source);

                if pkg_name != "context" {
                    continue;
                }
                if method_name != "Background" && method_name != "TODO" {
                    continue;
                }

                // Check enclosing function is not main or init
                let fn_name = Self::enclosing_function_name(call_node, source);
                match fn_name.as_deref() {
                    Some("main") | Some("init") => continue,
                    _ => {}
                }

                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "context_not_propagated".to_string(),
                    message: format!(
                        "context.{method_name}() in non-main function — propagate context from caller instead"
                    ),
                    snippet: extract_snippet(source, call_node, 1),
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
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ContextNotPropagatedPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_background_in_service_func() {
        let src = "package main\nfunc doWork() {\n\tctx := context.Background()\n\t_ = ctx\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "context_not_propagated");
    }

    #[test]
    fn detects_todo_in_service_func() {
        let src = "package main\nfunc doWork() {\n\tctx := context.TODO()\n\t_ = ctx\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_in_main() {
        let src = "package main\nfunc main() {\n\tctx := context.Background()\n\t_ = ctx\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_in_init() {
        let src = "package main\nfunc init() {\n\tctx := context.Background()\n\t_ = ctx\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
