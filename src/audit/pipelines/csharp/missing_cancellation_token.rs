use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_method_decl_query, extract_snippet, find_capture_index, has_modifier,
    is_csharp_suppressed, is_event_handler_signature, node_text,
};

pub struct MissingCancellationTokenPipeline {
    method_query: Arc<Query>,
}

impl MissingCancellationTokenPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_decl_query()?,
        })
    }
}

impl GraphPipeline for MissingCancellationTokenPipeline {
    fn name(&self) -> &str {
        "missing_cancellation_token"
    }

    fn description(&self) -> &str {
        "Detects async methods without a CancellationToken parameter"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let method_name_idx = find_capture_index(&self.method_query, "method_name");
        let method_decl_idx = find_capture_index(&self.method_query, "method_decl");
        let params_idx = find_capture_index(&self.method_query, "params");
        let return_type_idx = find_capture_index(&self.method_query, "return_type");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_name_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_decl_idx)
                .map(|c| c.node);
            let params_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == params_idx)
                .map(|c| c.node);
            let return_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == return_type_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(decl_node), Some(params_node)) =
                (name_node, decl_node, params_node)
            {
                // Only check async methods
                if !has_modifier(decl_node, source, "async") {
                    continue;
                }

                // Skip async void event handlers
                if let Some(ret) = return_node {
                    let ret_text = node_text(ret, source);
                    if ret_text == "void" && is_event_handler_signature(decl_node, source) {
                        continue;
                    }
                }

                // Skip controller action methods (CT available via model binding)
                if is_in_controller_class(decl_node, source) {
                    continue;
                }

                // Check suppression
                if is_csharp_suppressed(source, decl_node, "missing_cancellation_token") {
                    continue;
                }

                let method_name = node_text(name_node, source);

                // Check if any parameter is CancellationToken (including optional `= default`)
                if has_cancellation_token_param(params_node, source) {
                    continue;
                }

                // Severity: public → warning, private/protected → info
                let is_public =
                    has_modifier(decl_node, source, "public") || has_modifier(decl_node, source, "internal");
                let severity = if is_public { "warning" } else { "info" };

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "no_cancellation_token".to_string(),
                    message: format!(
                        "async method `{method_name}` has no CancellationToken parameter \u{2014} callers cannot cancel the operation"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        findings
    }
}

fn has_cancellation_token_param(params_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "parameter"
            && let Some(type_node) = child.child_by_field_name("type")
        {
            let type_text = node_text(type_node, source);
            // Match CancellationToken, CancellationToken? (nullable)
            if type_text == "CancellationToken" || type_text == "CancellationToken?" {
                return true;
            }
        }
    }
    false
}

/// Check if a method is inside a class ending with "Controller".
fn is_in_controller_class(method_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut current = Some(method_node);
    while let Some(n) = current {
        if n.kind() == "class_declaration" {
            if let Some(name_node) = n.child_by_field_name("name") {
                let name = name_node.utf8_text(source).unwrap_or("");
                if name.ends_with("Controller") {
                    return true;
                }
            }
        }
        current = n.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "Service.cs")
    }

    fn parse_and_check_with_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingCancellationTokenPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
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
    fn detects_async_without_ct() {
        let src = r#"
class Foo {
    public async Task DoWorkAsync() {
        await Task.Delay(1000);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "no_cancellation_token");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn clean_async_with_ct() {
        let src = r#"
class Foo {
    public async Task DoWorkAsync(CancellationToken cancellationToken) {
        await Task.Delay(1000, cancellationToken);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_sync_methods() {
        let src = r#"
class Foo {
    public void DoWork() { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = "class Foo { public async Task M() { } }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pipeline, "missing_cancellation_token");
    }

    #[test]
    fn event_handler_excluded() {
        let src = r#"
class Foo {
    async void OnClick(object sender, EventArgs e) {
        await Task.Delay(100);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn controller_action_excluded() {
        let src = r#"
class OrdersController {
    public async Task<IActionResult> GetOrders() {
        await Task.Delay(100);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn optional_ct_parameter_clean() {
        let src = r#"
class Foo {
    public async Task DoWork(CancellationToken ct = default) {
        await Task.Delay(100);
    }
}
"#;
        // The parameter type is CancellationToken even with `= default`
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn public_is_warning_private_is_info() {
        let src = r#"
class Foo {
    public async Task PublicMethod() { }
    private async Task PrivateMethod() { }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        let public_f = findings.iter().find(|f| f.message.contains("PublicMethod")).unwrap();
        let private_f = findings.iter().find(|f| f.message.contains("PrivateMethod")).unwrap();
        assert_eq!(public_f.severity, "warning");
        assert_eq!(private_f.severity, "info");
    }

    #[test]
    fn excluded_in_test_files() {
        let src = r#"
class FooTests {
    public async Task TestWork() { }
}
"#;
        let findings = parse_and_check_with_path(src, "FooTests.cs");
        assert!(findings.is_empty());
    }
}
