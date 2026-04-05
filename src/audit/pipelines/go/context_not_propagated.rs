use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{
    is_go_constructor, is_generated_go_file, is_nolint_suppressed, is_test_context_go,
};

use super::primitives::{
    compile_selector_call_query, extract_snippet, find_capture_index, node_text,
};

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
            if parent.kind() == "function_declaration"
                && let Some(name_node) = parent.child_by_field_name("name")
            {
                return Some(node_text(name_node, source).to_string());
            }
            current = parent.parent();
        }
        None
    }

    fn enclosing_function_has_context_param(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_declaration" || parent.kind() == "method_declaration" {
                if let Some(params) = parent.child_by_field_name("parameters") {
                    let count = params.named_child_count();
                    for i in 0..count {
                        if let Some(param) = params.named_child(i) {
                            if param.kind() == "parameter_declaration" {
                                if let Some(type_node) = param.child_by_field_name("type") {
                                    let type_text = node_text(type_node, source);
                                    if type_text.contains("context.Context")
                                        || type_text == "Context"
                                    {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
                return false;
            }
            current = parent.parent();
        }
        false
    }
}

impl GraphPipeline for ContextNotPropagatedPipeline {
    fn name(&self) -> &str {
        "context_not_propagated"
    }

    fn description(&self) -> &str {
        "Detects context.Background()/context.TODO() in non-main/non-init functions"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_generated_go_file(file_path, source) {
            return vec![];
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let pkg_idx = find_capture_index(&self.call_query, "pkg");
        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let pkg_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == pkg_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(pkg_node), Some(method_node), Some(call_node)) =
                (pkg_node, method_node, call_node)
            {
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

                // Skip in test functions or _test.go files
                if is_test_context_go(call_node, source, file_path) {
                    continue;
                }

                // Skip in New*/Init*/Setup*/Start*/Run*/Serve* functions (service bootstrapping)
                if is_go_constructor(call_node, source) {
                    continue;
                }
                if let Some(ref name) = fn_name {
                    if name.starts_with("Setup")
                        || name.starts_with("Start")
                        || name.starts_with("Run")
                        || name.starts_with("Serve")
                    {
                        continue;
                    }
                }

                // Check nolint suppression
                if is_nolint_suppressed(source, call_node, self.name()) {
                    continue;
                }

                // Severity graduation based on whether enclosing function has a context param
                let has_ctx_param =
                    Self::enclosing_function_has_context_param(call_node, source);
                let severity = if has_ctx_param && method_name == "Background" {
                    "error"
                } else if has_ctx_param && method_name == "TODO" {
                    "warning"
                } else {
                    "info"
                };

                let start = call_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: format!("context_{}_in_func", method_name.to_lowercase()),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_file(source, "test.go")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ContextNotPropagatedPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_background_in_service_func() {
        let src = "package main\nfunc doWork() {\n\tctx := context.Background()\n\t_ = ctx\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "context_background_in_func");
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

    #[test]
    fn setup_function_not_flagged() {
        let src = "package main\nfunc SetupRoutes() {\n\tctx := context.Background()\n\t_ = ctx\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn start_function_not_flagged() {
        let src = "package main\nfunc StartServer() {\n\tctx := context.Background()\n\t_ = ctx\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn function_with_context_param_creating_background_is_error() {
        let src = "package main\nfunc Handle(ctx context.Context) {\n\tbg := context.Background()\n\t_ = bg\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn function_without_context_param_is_info() {
        let src = "package main\nfunc helper() {\n\tbg := context.Background()\n\t_ = bg\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn nolint_suppression_skips_finding() {
        let src = "package main\nfunc helper() {\n\tbg := context.Background() // NOLINT(context_not_propagated)\n\t_ = bg\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc helper() {\n\tbg := context.Background()\n\t_ = bg\n}\n";
        let findings = parse_and_check_file(src, "service.pb.go");
        assert!(findings.is_empty());
    }
}
