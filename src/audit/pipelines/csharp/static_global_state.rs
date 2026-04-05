use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_field_decl_query, compile_property_decl_query, extract_snippet, find_capture_index,
    has_csharp_attribute, has_modifier, is_csharp_suppressed, node_text,
};

pub struct StaticGlobalStatePipeline {
    field_query: Arc<Query>,
    property_query: Arc<Query>,
}

impl StaticGlobalStatePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            field_query: compile_field_decl_query()?,
            property_query: compile_property_decl_query()?,
        })
    }
}

impl GraphPipeline for StaticGlobalStatePipeline {
    fn name(&self) -> &str {
        "static_global_state"
    }

    fn description(&self) -> &str {
        "Detects mutable static fields and properties that create hidden global state"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check static fields
        self.check_fields(tree, source, file_path, &mut findings);

        // Check static properties with { get; set; }
        self.check_properties(tree, source, file_path, &mut findings);

        findings
    }
}

impl StaticGlobalStatePipeline {
    fn check_fields(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.field_query, tree.root_node(), source);

        let field_decl_idx = find_capture_index(&self.field_query, "field_decl");
        let field_name_idx = find_capture_index(&self.field_query, "field_name");

        while let Some(m) = matches.next() {
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_decl_idx)
                .map(|c| c.node);
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == field_name_idx)
                .map(|c| c.node);

            if let (Some(decl_node), Some(name_node)) = (decl_node, name_node)
                && has_modifier(decl_node, source, "static")
                && !has_modifier(decl_node, source, "readonly")
                && !has_modifier(decl_node, source, "const")
            {
                // Skip [ThreadStatic] fields — per-thread, not truly global
                if has_csharp_attribute(decl_node, source, "ThreadStatic") {
                    continue;
                }

                // Check suppression
                if is_csharp_suppressed(source, decl_node, "static_global_state") {
                    continue;
                }

                let field_name = node_text(name_node, source);
                let is_public = has_modifier(decl_node, source, "public")
                    || has_modifier(decl_node, source, "internal");

                // Severity: public/internal static → error, private/protected → warning
                let severity = if is_public { "error" } else { "warning" };

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "mutable_static_field".to_string(),
                    message: format!(
                        "mutable static field `{field_name}` creates hidden global state \u{2014} consider readonly, const, or dependency injection"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }
    }

    fn check_properties(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.property_query, tree.root_node(), source);

        let prop_decl_idx = find_capture_index(&self.property_query, "prop_decl");
        let prop_name_idx = find_capture_index(&self.property_query, "prop_name");

        while let Some(m) = matches.next() {
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == prop_decl_idx)
                .map(|c| c.node);
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == prop_name_idx)
                .map(|c| c.node);

            if let (Some(decl_node), Some(name_node)) = (decl_node, name_node)
                && has_modifier(decl_node, source, "static")
            {
                // Check if property has a setter (mutable)
                let prop_text = decl_node.utf8_text(source).unwrap_or("");
                if !prop_text.contains("set;") && !prop_text.contains("set {") {
                    continue;
                }

                if is_csharp_suppressed(source, decl_node, "static_global_state") {
                    continue;
                }

                let prop_name = node_text(name_node, source);
                let is_public = has_modifier(decl_node, source, "public")
                    || has_modifier(decl_node, source, "internal");
                let severity = if is_public { "error" } else { "warning" };

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "mutable_static_property".to_string(),
                    message: format!(
                        "mutable static property `{prop_name}` creates hidden global state \u{2014} consider removing the setter or using dependency injection"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }
    }
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
        let pipeline = StaticGlobalStatePipeline::new().unwrap();
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
    fn detects_mutable_static() {
        let src = r#"
class Foo {
    private static int _counter;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_static_field");
    }

    #[test]
    fn clean_static_readonly() {
        let src = r#"
class Foo {
    private static readonly int MaxSize = 100;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_const() {
        let src = r#"
class Foo {
    const int MaxSize = 100;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_instance_field() {
        let src = r#"
class Foo {
    private int _counter;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = "class Foo { public static string Instance; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pipeline, "static_global_state");
    }

    #[test]
    fn public_static_is_error_severity() {
        let src = r#"
class Foo {
    public static int Counter;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn private_static_is_warning_severity() {
        let src = r#"
class Foo {
    private static int _counter;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn threadstatic_excluded() {
        let src = r#"
class Foo {
    [ThreadStatic]
    private static int _perThread;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_mutable_static_property() {
        let src = r#"
class Foo {
    public static int Counter { get; set; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_static_property");
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn clean_static_readonly_property() {
        let src = r#"
class Foo {
    public static int Counter { get; }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn excluded_in_test_files() {
        let src = r#"
class FooTests {
    private static int _counter;
}
"#;
        let findings = parse_and_check_with_path(src, "FooTests.cs");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppressed_by_nolint() {
        let src = r#"
class Foo {
    // NOLINT
    private static int _counter;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
