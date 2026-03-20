use std::sync::Arc;

use anyhow::Result;
use tree_sitter::{Query, Tree};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
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

impl Pipeline for GodObjectDetectionPipeline {
    fn name(&self) -> &str {
        "god_object_detection"
    }

    fn description(&self) -> &str {
        "Detects impl blocks with too many methods and structs with too many fields, indicating a type is doing too much and should be decomposed"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        let impl_matches = primitives::find_large_impl_blocks(
            tree,
            source,
            &self.impl_query,
            LARGE_IMPL_THRESHOLD,
        );

        for m in impl_matches {
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
            if let Some(n) = impl_node {
                // Walk up to find the impl_item
                let mut current = Some(n);
                while let Some(c) = current {
                    if c.kind() == "impl_item" {
                        if is_trait_impl(c, source) {
                            break;
                        }
                        // Not a trait impl, fall through to flag it
                        break;
                    }
                    current = c.parent();
                }
                if let Some(c) = current
                    && c.kind() == "impl_item"
                    && is_trait_impl(c, source)
                {
                    continue;
                }
            }
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: m.line,
                column: m.column,
                severity: "warning".to_string(),
                pipeline: self.name().to_string(),
                pattern: "large_impl_block".to_string(),
                message: format!(
                    "impl block for `{}` has {} methods (threshold: {}) — consider splitting into smaller traits or extracting helper types",
                    m.name, m.child_count, LARGE_IMPL_THRESHOLD
                ),
                snippet: m.snippet,
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
        pipeline.check(&tree, source.as_bytes(), "test.rs")
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
        assert_eq!(f.severity, "warning");
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
    fn multiple_impl_blocks_counted_separately() {
        let src = format!(
            "struct Foo;\nimpl Foo {{\n{}\n}}\nimpl Foo {{\n{}\n}}",
            gen_methods(10),
            gen_methods(3)
        );
        let findings = parse_and_check(&src);
        // Only the first impl (10 methods) should be flagged, not the second (3 methods)
        assert_eq!(findings.len(), 1);
    }
}
