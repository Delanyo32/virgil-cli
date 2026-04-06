use anyhow::Result;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_noqa_suppressed;

use super::primitives::{extract_snippet, node_text};

const NESTING_THRESHOLD: usize = 4;
const NESTING_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "while_statement",
    "with_statement",
    "try_statement",
];

pub struct DeepNestingPipeline;

impl DeepNestingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Find the enclosing function name for a node.
    fn enclosing_function_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_definition"
                && let Some(name_node) = parent.child_by_field_name("name")
            {
                return Some(node_text(name_node, source).to_string());
            }
            current = parent.parent();
        }
        None
    }

    /// Check if a with_statement's only block child is another with_statement (adjacent with).
    fn is_adjacent_with(node: tree_sitter::Node) -> bool {
        if node.kind() != "with_statement" {
            return false;
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i)
                && child.kind() == "block"
            {
                let named_count = child.named_child_count();
                if named_count == 1
                    && let Some(inner) = child.named_child(0)
                {
                    return inner.kind() == "with_statement";
                }
            }
        }
        false
    }

    fn walk_tree(
        node: tree_sitter::Node,
        depth: usize,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let is_nesting = NESTING_KINDS.contains(&node.kind());

        // Collapse adjacent with statements: `with a:\n    with b:` counts as one level
        let collapsed = is_nesting && node.kind() == "with_statement" && Self::is_adjacent_with(node);
        let new_depth = if is_nesting && !collapsed {
            depth + 1
        } else {
            depth
        };

        if new_depth > NESTING_THRESHOLD && is_nesting && !collapsed {
            if is_noqa_suppressed(source, node, "deep_nesting") {
                return;
            }

            // Severity graduation based on depth
            let severity = if new_depth >= 8 {
                "critical"
            } else if new_depth >= 6 {
                "error"
            } else {
                "warning"
            };

            let fn_context = Self::enclosing_function_name(node, source)
                .map(|name| format!(" in `{name}`"))
                .unwrap_or_default();

            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: severity.to_string(),
                pipeline: "deep_nesting".to_string(),
                pattern: "excessive_nesting_depth".to_string(),
                message: format!(
                    "nesting depth {new_depth}{fn_context} exceeds threshold ({NESTING_THRESHOLD}) — consider early returns or extracting helpers"
                ),
                snippet: extract_snippet(source, node, 2),
            });
            // Stop recursing this branch to avoid duplicate reports
            return;
        }

        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            Self::walk_tree(child, new_depth, source, file_path, findings);
        }
    }
}

impl GraphPipeline for DeepNestingPipeline {
    fn name(&self) -> &str {
        "deep_nesting"
    }

    fn description(&self) -> &str {
        "Detects deeply nested control flow (>4 levels) — arrow anti-pattern"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut findings = Vec::new();
        Self::walk_tree(tree.root_node(), 0, source, file_path, &mut findings);
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeepNestingPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_deep_nesting() {
        let src = "\
def foo():
    if True:
        if True:
            if True:
                if True:
                    if True:
                        pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "excessive_nesting_depth");
    }

    #[test]
    fn clean_shallow_nesting() {
        let src = "\
def foo():
    if True:
        if True:
            if True:
                if True:
                    pass
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_mixed_control_flow() {
        let src = "\
def foo():
    for x in items:
        if x:
            while True:
                with ctx:
                    if y:
                        pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_flat_function() {
        let src = "def foo():\n    x = 1\n    y = 2\n    return x + y\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn severity_warning_at_depth_5() {
        let src = "\
def foo():
    if True:
        if True:
            if True:
                if True:
                    if True:
                        pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn message_includes_function_name() {
        let src = "\
def process_data():
    if True:
        if True:
            if True:
                if True:
                    if True:
                        pass
";
        let findings = parse_and_check(src);
        assert!(
            findings[0].message.contains("process_data"),
            "message should include function name"
        );
    }

    #[test]
    fn noqa_suppresses() {
        let src = "\
def foo():
    if True:
        if True:
            if True:
                if True:
                    if True:  # noqa
                        pass
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "# noqa should suppress");
    }
}
