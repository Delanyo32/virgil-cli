use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_class_decl_query, extract_snippet, find_capture_index, has_csharp_attribute,
    has_modifier, is_csharp_suppressed, node_text,
};

const MAX_ACTIONS: usize = 8;

pub struct GodControllerPipeline {
    class_query: Arc<Query>,
}

impl GodControllerPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_decl_query()?,
        })
    }
}

impl GraphPipeline for GodControllerPipeline {
    fn name(&self) -> &str {
        "god_controller"
    }

    fn description(&self) -> &str {
        "Detects Controller classes with too many action methods"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let class_name_idx = find_capture_index(&self.class_query, "class_name");
        let class_body_idx = find_capture_index(&self.class_query, "class_body");
        let class_decl_idx = find_capture_index(&self.class_query, "class_decl");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_body_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_decl_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(body_node), Some(decl_node)) =
                (name_node, body_node, decl_node)
            {
                let class_name = node_text(name_node, source);

                // Check: name ends with "Controller" OR has [ApiController] attribute
                let is_controller = class_name.ends_with("Controller")
                    || has_csharp_attribute(decl_node, source, "ApiController");

                if !is_controller {
                    continue;
                }

                if is_csharp_suppressed(source, decl_node, "god_controller") {
                    continue;
                }

                // Count public methods excluding [NonAction] decorated ones
                let mut action_count = 0;
                let mut body_cursor = body_node.walk();
                for child in body_node.children(&mut body_cursor) {
                    if child.kind() == "method_declaration"
                        && has_modifier(child, source, "public")
                        && !has_csharp_attribute(child, source, "NonAction")
                    {
                        action_count += 1;
                    }
                }

                if action_count > MAX_ACTIONS {
                    let over = action_count - MAX_ACTIONS;
                    let severity = if over > 12 {
                        "error"
                    } else if over > 4 {
                        "warning"
                    } else {
                        "info"
                    };

                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "oversized_controller".to_string(),
                        message: format!(
                            "controller `{class_name}` has {action_count} action methods (>{MAX_ACTIONS}) \u{2014} consider splitting into multiple controllers"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
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
            .set_language(&Language::CSharp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GodControllerPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "Controllers.cs",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_oversized_controller() {
        let methods: Vec<String> = (0..10)
            .map(|i| format!("public IActionResult Action{i}() {{ return Ok(); }}"))
            .collect();
        let src = format!("class OrdersController {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "oversized_controller");
    }

    #[test]
    fn clean_small_controller() {
        let src = r#"
class OrdersController {
    public IActionResult Index() { return Ok(); }
    public IActionResult Details(int id) { return Ok(); }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_controller_classes() {
        let methods: Vec<String> = (0..20)
            .map(|i| format!("public void M{i}() {{ }}"))
            .collect();
        let src = format!("class OrderService {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn counts_only_public_methods() {
        let mut methods = Vec::new();
        for i in 0..5 {
            methods.push(format!("public IActionResult Pub{i}() {{ return Ok(); }}"));
        }
        for i in 0..10 {
            methods.push(format!("private void Priv{i}() {{ }}"));
        }
        let src = format!("class FooController {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert!(findings.is_empty()); // only 5 public
    }

    #[test]
    fn non_action_excluded_from_count() {
        let mut methods = Vec::new();
        for i in 0..10 {
            methods.push(format!("public IActionResult Action{i}() {{ return Ok(); }}"));
        }
        // Add 3 NonAction methods — should not be counted
        for i in 0..3 {
            methods.push(format!(
                "[NonAction] public void Helper{i}() {{ }}"
            ));
        }
        let src = format!("class FooController {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        // Message should show 10 actions, not 13
        assert!(findings[0].message.contains("10 action methods"));
    }

    #[test]
    fn severity_graduation() {
        // 9 actions → info (1 over)
        let methods: Vec<String> = (0..9)
            .map(|i| format!("public void M{i}() {{ }}"))
            .collect();
        let src = format!("class FooController {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");

        // 15 actions → warning (7 over)
        let methods: Vec<String> = (0..15)
            .map(|i| format!("public void M{i}() {{ }}"))
            .collect();
        let src = format!("class FooController {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings[0].severity, "warning");

        // 25 actions → error (17 over)
        let methods: Vec<String> = (0..25)
            .map(|i| format!("public void M{i}() {{ }}"))
            .collect();
        let src = format!("class FooController {{ {} }}", methods.join("\n"));
        let findings = parse_and_check(&src);
        assert_eq!(findings[0].severity, "error");
    }
}
