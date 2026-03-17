use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use super::primitives::{self, extract_snippet, find_capture_index, node_text};

pub struct TypeConfusionPipeline {
    union_query: Arc<Query>,
}

impl TypeConfusionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            union_query: primitives::compile_union_item_query()?,
        })
    }
}

impl Pipeline for TypeConfusionPipeline {
    fn name(&self) -> &str {
        "type_confusion"
    }

    fn description(&self) -> &str {
        "Detects type confusion risks: transmute calls and union field access"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Find all transmute calls by walking the tree
        // This handles both plain (std::mem::transmute()) and turbofish
        // (std::mem::transmute::<T, U>()) syntax
        {
            let mut stack = vec![tree.root_node()];
            while let Some(node) = stack.pop() {
                if node.kind() == "call_expression" {
                    if let Some(func) = node.child_by_field_name("function") {
                        let func_text = node_text(func, source);
                        if func_text.contains("transmute") {
                            let start = node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "transmute_call".to_string(),
                                message: "mem::transmute bypasses type safety".to_string(),
                                snippet: extract_snippet(source, node, 1),
                            });
                            continue;
                        }
                    }
                }
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        stack.push(child);
                    }
                }
            }
        }

        // Find all union definitions
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.union_query, tree.root_node(), source);

            let union_name_idx = find_capture_index(&self.union_query, "union_name");
            let union_def_idx = find_capture_index(&self.union_query, "union_def");

            while let Some(m) = matches.next() {
                let name_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == union_name_idx)
                    .map(|c| c.node);
                let def_cap = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == union_def_idx)
                    .map(|c| c.node);

                if let (Some(name_n), Some(def_n)) = (name_cap, def_cap) {
                    let union_name = node_text(name_n, source);
                    let start = def_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "error".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "union_field_access".to_string(),
                        message: format!(
                            "union type `{union_name}` enables type confusion through field access"
                        ),
                        snippet: extract_snippet(source, def_n, 3),
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
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TypeConfusionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_transmute_call() {
        let src = r#"fn f() { unsafe { std::mem::transmute::<u32, f32>(42); } }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "transmute_call");
    }

    #[test]
    fn detects_union_definition() {
        let src = r#"union MyUnion { i: i32, f: f32 }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "union_field_access");
    }

    #[test]
    fn clean_no_findings() {
        let src = r#"fn f() { let x: i32 = 42; }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
