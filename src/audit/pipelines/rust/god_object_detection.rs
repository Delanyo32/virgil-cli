use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tree_sitter::Query;

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_trait_impl;

const LARGE_IMPL_THRESHOLD: usize = 10;
const LARGE_STRUCT_THRESHOLD: usize = 15;

pub struct GodObjectDetectionPipeline {
    impl_query: Arc<Query>,
    struct_query: Arc<Query>,
}

impl GodObjectDetectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            impl_query: primitives::compile_impl_block_query()?,
            struct_query: primitives::compile_struct_fields_query()?,
        })
    }
}

impl GraphPipeline for GodObjectDetectionPipeline {
    fn name(&self) -> &str {
        "god_object_detection"
    }

    fn description(&self) -> &str {
        "Detects impl blocks with too many methods and structs with too many fields, indicating a type is doing too much and should be decomposed"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut findings = Vec::new();

        // Get ALL impl blocks (threshold=1) and group by type name
        let all_impl_matches = primitives::find_large_impl_blocks(
            tree,
            source,
            &self.impl_query,
            1,
        );

        // Aggregate method counts by type name
        let mut type_methods: HashMap<String, (usize, u32, u32, String)> = HashMap::new();
        for m in &all_impl_matches {
            // Skip trait impls — they cannot be split; flagging is not actionable
            let impl_node = tree.root_node().descendant_for_point_range(
                tree_sitter::Point {
                    row: (m.line - 1) as usize,
                    column: (m.column - 1) as usize,
                },
                tree_sitter::Point {
                    row: (m.line - 1) as usize,
                    column: (m.column - 1) as usize,
                },
            );
            let mut skip = false;
            if let Some(n) = impl_node {
                let mut current = Some(n);
                while let Some(c) = current {
                    if c.kind() == "impl_item" {
                        if is_trait_impl(c, source) {
                            skip = true;
                        }
                        break;
                    }
                    current = c.parent();
                }
            }
            if skip {
                continue;
            }

            let entry = type_methods.entry(m.name.clone()).or_insert((0, m.line, m.column, m.snippet.clone()));
            entry.0 += m.child_count;
        }

        for (type_name, (total_methods, line, column, snippet)) in type_methods {
            if total_methods < LARGE_IMPL_THRESHOLD {
                continue;
            }

            // Skip Builder and Config types — they commonly have many methods by design
            if type_name.ends_with("Builder") || type_name.ends_with("Config") {
                continue;
            }

            let severity = if total_methods >= 20 {
                "error"
            } else if total_methods >= 15 {
                "warning"
            } else {
                "info"
            };

            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line,
                column,
                severity: severity.to_string(),
                pipeline: self.name().to_string(),
                pattern: "large_impl_block".to_string(),
                message: format!(
                    "impl block for `{}` has {} methods (threshold: {}) — consider splitting into smaller traits or extracting helper types",
                    type_name, total_methods, LARGE_IMPL_THRESHOLD
                ),
                snippet,
            });
        }

        let struct_matches = primitives::find_large_structs(
            tree,
            source,
            &self.struct_query,
            LARGE_STRUCT_THRESHOLD,
        );

        for m in struct_matches {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: m.line,
                column: m.column,
                severity: "warning".to_string(),
                pipeline: self.name().to_string(),
                pattern: "large_struct".to_string(),
                message: format!(
                    "struct `{}` has {} fields (threshold: {}) — consider grouping related fields into sub-structs",
                    m.name, m.child_count, LARGE_STRUCT_THRESHOLD
                ),
                snippet: m.snippet,
            });
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GodObjectDetectionPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.rs",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    fn gen_methods(n: usize) -> String {
        (0..n)
            .map(|i| format!("    fn method_{}(&self) {{}}", i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn gen_fields(n: usize) -> String {
        (0..n)
            .map(|i| format!("    field_{}: i32,", i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn detects_large_impl_block() {
        let src = format!("struct Foo;\nimpl Foo {{\n{}\n}}", gen_methods(12));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "large_impl_block");
        assert!(findings[0].message.contains("12 methods"));
    }

    #[test]
    fn does_not_flag_small_impl() {
        let src = format!("struct Foo;\nimpl Foo {{\n{}\n}}", gen_methods(3));
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_large_struct() {
        let src = format!("struct BigStruct {{\n{}\n}}", gen_fields(16));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "large_struct");
        assert!(findings[0].message.contains("16 fields"));
    }

    #[test]
    fn does_not_flag_small_struct() {
        let src = format!("struct SmallStruct {{\n{}\n}}", gen_fields(5));
        let findings = parse_and_check(&src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_both_in_same_file() {
        let src = format!(
            "struct BigStruct {{\n{}\n}}\nimpl BigStruct {{\n{}\n}}",
            gen_fields(16),
            gen_methods(11)
        );
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 2);
        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"large_impl_block"));
        assert!(patterns.contains(&"large_struct"));
    }

    #[test]
    fn trait_impl_skipped() {
        let src = format!(
            "struct Foo;\ntrait BigTrait {{}}\nimpl BigTrait for Foo {{\n{}\n}}",
            gen_methods(10)
        );
        let findings = parse_and_check(&src);
        // Trait impls cannot be split, so they should not be flagged
        assert!(findings.is_empty());
    }

    #[test]
    fn generic_struct_detected() {
        let fields = (0..16)
            .map(|i| format!("    field_{}: T,", i))
            .collect::<Vec<_>>()
            .join("\n");
        let src = format!("struct Generic<T> {{\n{}\n}}", fields);
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "large_struct");
        assert!(findings[0].message.contains("Generic"));
    }

    #[test]
    fn clean_code_no_findings() {
        let src = r#"
struct Small {
    a: i32,
    b: String,
}

impl Small {
    fn new() -> Self { Self { a: 0, b: String::new() } }
    fn get_a(&self) -> i32 { self.a }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn correct_metadata() {
        let src = format!("struct Foo;\nimpl Foo {{\n{}\n}}", gen_methods(10));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "god_object_detection");
        assert_eq!(f.pattern, "large_impl_block");
    }

    #[test]
    fn snippet_is_short() {
        let src = format!("struct Foo;\nimpl Foo {{\n{}\n}}", gen_methods(12));
        let findings = parse_and_check(&src);
        assert_eq!(findings.len(), 1);
        // Snippet should be truncated (3 lines + ...)
        assert!(findings[0].snippet.contains("..."));
        let lines: Vec<&str> = findings[0].snippet.lines().collect();
        assert!(lines.len() <= 4); // 3 content lines + "..."
    }

    #[test]
    fn empty_impl_no_findings() {
        let src = "struct Foo;\nimpl Foo {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn two_impl_blocks_aggregated() {
        // 6 methods each block = 12 total, exceeds threshold of 10
        let src = "struct Foo {}\nimpl Foo { fn a(&self){} fn b(&self){} fn c(&self){} fn d(&self){} fn e(&self){} fn f(&self){} }\nimpl Foo { fn g(&self){} fn h(&self){} fn i(&self){} fn j(&self){} fn k(&self){} fn l(&self){} }";
        let findings = parse_and_check(src);
        assert!(!findings.is_empty(), "12 methods across 2 impl blocks should be flagged");
    }

    #[test]
    fn builder_type_exempt() {
        // 12 methods but it's a Builder
        let src = "struct FooBuilder {}\nimpl FooBuilder { fn a(&self){} fn b(&self){} fn c(&self){} fn d(&self){} fn e(&self){} fn f(&self){} fn g(&self){} fn h(&self){} fn i(&self){} fn j(&self){} fn k(&self){} fn l(&self){} }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "Builder types should be exempt");
    }

    #[test]
    fn severity_error_at_20_methods() {
        let methods: Vec<String> = (0..20).map(|i| format!("fn m{}(&self){{}}", i)).collect();
        let src = format!("struct Foo {{}}\nimpl Foo {{ {} }}", methods.join(" "));
        let findings = parse_and_check(&src);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, "error");
    }
}
