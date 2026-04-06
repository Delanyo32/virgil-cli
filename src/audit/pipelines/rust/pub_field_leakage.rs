use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{has_attribute_text, is_test_file, struct_has_derive};

pub struct PubFieldLeakagePipeline {
    struct_query: Arc<Query>,
    impl_query: Arc<Query>,
}

impl PubFieldLeakagePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            struct_query: primitives::compile_struct_fields_query()?,
            impl_query: primitives::compile_impl_block_query()?,
        })
    }

    fn is_struct_pub(struct_node: tree_sitter::Node) -> bool {
        (0..struct_node.named_child_count())
            .filter_map(|i| struct_node.named_child(i))
            .any(|c| c.kind() == "visibility_modifier")
    }

    fn has_visibility_modifier(field_node: tree_sitter::Node) -> bool {
        (0..field_node.named_child_count())
            .filter_map(|i| field_node.named_child(i))
            .any(|c| c.kind() == "visibility_modifier")
    }
}

fn has_impl_methods(
    tree: &tree_sitter::Tree,
    source: &[u8],
    impl_query: &Query,
    struct_name: &str,
) -> bool {
    use streaming_iterator::StreamingIterator;
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(impl_query, tree.root_node(), source);
    let type_idx = impl_query
        .capture_names()
        .iter()
        .position(|n| *n == "type_name")
        .unwrap_or(0);
    let body_idx = impl_query
        .capture_names()
        .iter()
        .position(|n| *n == "body")
        .unwrap_or(1);
    while let Some(m) = matches.next() {
        let type_text = m
            .captures
            .iter()
            .find(|c| c.index as usize == type_idx)
            .map(|c| c.node.utf8_text(source).unwrap_or(""))
            .unwrap_or("");
        if type_text == struct_name {
            if let Some(body_cap) = m.captures.iter().find(|c| c.index as usize == body_idx) {
                for i in 0..body_cap.node.child_count() {
                    if body_cap
                        .node
                        .child(i)
                        .map_or(false, |c| c.kind() == "function_item")
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

impl GraphPipeline for PubFieldLeakagePipeline {
    fn name(&self) -> &str {
        "pub_field_leakage"
    }

    fn description(&self) -> &str {
        "Detects pub structs where all fields are pub, preventing future invariant enforcement — consider encapsulating with accessor methods"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        if is_test_file(file_path) {
            return vec![];
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.struct_query, tree.root_node(), source);

        let name_idx = self
            .struct_query
            .capture_names()
            .iter()
            .position(|n| *n == "struct_name")
            .unwrap();
        let fields_idx = self
            .struct_query
            .capture_names()
            .iter()
            .position(|n| *n == "fields")
            .unwrap();
        let struct_idx = self
            .struct_query
            .capture_names()
            .iter()
            .position(|n| *n == "struct_def")
            .unwrap();

        while let Some(m) = matches.next() {
            let name_node = m.captures.iter().find(|c| c.index as usize == name_idx);
            let fields_node = m.captures.iter().find(|c| c.index as usize == fields_idx);
            let struct_node = m.captures.iter().find(|c| c.index as usize == struct_idx);

            if let (Some(name_cap), Some(fields_cap), Some(struct_cap)) =
                (name_node, fields_node, struct_node)
            {
                // Only flag pub structs
                if !Self::is_struct_pub(struct_cap.node) {
                    continue;
                }

                let struct_name = name_cap.node.utf8_text(source).unwrap_or("");

                // Skip Config/Settings/Options/Args/Props/Params structs
                let name_lower = struct_name.to_lowercase();
                if ["config", "settings", "options", "args", "props", "params"]
                    .iter()
                    .any(|s| name_lower.ends_with(s))
                {
                    continue;
                }

                let field_decls: Vec<_> = (0..fields_cap.node.named_child_count())
                    .filter_map(|i| fields_cap.node.named_child(i))
                    .filter(|child| child.kind() == "field_declaration")
                    .collect();

                let total = field_decls.len();
                if total == 0 {
                    continue;
                }

                // Skip small structs (2 or fewer fields — no meaningful invariant)
                if total <= 2 {
                    continue;
                }

                // Skip serde DTOs
                if struct_has_derive(struct_cap.node, source, "Deserialize")
                    || struct_has_derive(struct_cap.node, source, "Serialize")
                {
                    continue;
                }

                // Skip FFI structs
                if has_attribute_text(struct_cap.node, source, "repr(C)") {
                    continue;
                }

                // Skip structs that have impl methods — they have behavioural encapsulation already
                if has_impl_methods(tree, source, &self.impl_query, struct_name) {
                    continue;
                }

                let pub_count = field_decls
                    .iter()
                    .filter(|f| Self::has_visibility_modifier(**f))
                    .count();

                // Flag when every field is pub — struct can't enforce any invariants
                if pub_count == total {
                    let start = struct_cap.node.start_position();
                    let snippet = primitives::extract_snippet(source, struct_cap.node, 3);
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "all_fields_public".to_string(),
                        message: format!(
                            "pub struct `{struct_name}` has all {total} fields public — can't add validation later; consider encapsulating with private fields and accessor methods"
                        ),
                        snippet,
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
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "test.rs")
    }

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PubFieldLeakagePipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn skips_small_struct_two_fields() {
        // Structs with <= 2 fields are now exempt
        let src = r#"
pub struct Account {
    pub balance: i64,
    pub status: String,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_mixed_visibility() {
        let src = r#"
pub struct Account {
    balance: i64,
    pub name: String,
    pub extra: bool,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_all_private_fields() {
        let src = r#"
pub struct Account {
    balance: i64,
    status: String,
    extra: bool,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_pub_struct() {
        let src = r#"
struct Internal {
    pub a: i32,
    pub b: String,
    pub c: bool,
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_larger_fully_public_struct() {
        let src = r#"
pub struct Leaky {
    pub host: String,
    pub port: u16,
    pub timeout: u64,
    pub retries: u32,
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("all 4 fields public"));
    }

    #[test]
    fn correct_metadata() {
        let src = r#"
pub struct Leaky {
    pub a: i32,
    pub b: String,
    pub c: bool,
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "pub_field_leakage");
        assert_eq!(f.severity, "info");
    }

    #[test]
    fn test_file_excluded() {
        let src =
            r#"pub struct Foo { pub a: i32, pub b: String, pub c: bool, pub d: u64 }"#;
        let findings = parse_and_check_path(src, "tests/fixtures.rs");
        assert!(findings.is_empty());
    }

    #[test]
    fn config_struct_not_flagged() {
        let src = r#"pub struct AppConfig { pub host: String, pub port: u16, pub timeout: u64, pub retries: u32 }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "Config structs should be exempt");
    }

    #[test]
    fn struct_with_methods_not_flagged() {
        let src = r#"
pub struct Foo { pub a: i32, pub b: String, pub c: bool, pub d: u64 }
impl Foo { fn new() -> Self { Foo { a: 0, b: String::new(), c: false, d: 0 } } }
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "struct with impl methods should not be flagged");
    }
}
