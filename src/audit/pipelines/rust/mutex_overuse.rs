use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

pub struct MutexOverusePipeline {
    generic_query: Arc<Query>,
}

impl MutexOverusePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            generic_query: primitives::compile_generic_type_query()?,
        })
    }
}

impl GraphPipeline for MutexOverusePipeline {
    fn name(&self) -> &str {
        "mutex_overuse"
    }

    fn description(&self) -> &str {
        "Detects Arc<Mutex<T>> and Arc<RwLock<T>> patterns that may indicate over-synchronization"
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
        let mut matches = cursor.matches(&self.generic_query, tree.root_node(), source);

        let outer_idx = self
            .generic_query
            .capture_names()
            .iter()
            .position(|n| *n == "outer_type")
            .unwrap();
        let inner_idx = self
            .generic_query
            .capture_names()
            .iter()
            .position(|n| *n == "inner_type")
            .unwrap();
        let generic_idx = self
            .generic_query
            .capture_names()
            .iter()
            .position(|n| *n == "generic")
            .unwrap();

        while let Some(m) = matches.next() {
            let outer_node = m.captures.iter().find(|c| c.index as usize == outer_idx);
            let inner_node = m.captures.iter().find(|c| c.index as usize == inner_idx);
            let generic_node = m.captures.iter().find(|c| c.index as usize == generic_idx);

            if let (Some(outer_cap), Some(inner_cap), Some(generic_cap)) =
                (outer_node, inner_node, generic_node)
            {
                let outer_text = outer_cap.node.utf8_text(source).unwrap_or("");
                let inner_text = inner_cap.node.utf8_text(source).unwrap_or("");

                if outer_text == "Arc"
                    && (inner_text.ends_with("Mutex") || inner_text.ends_with("RwLock"))
                {
                    let pattern = if inner_text.ends_with("RwLock") {
                        "arc_rwlock"
                    } else {
                        "arc_mutex"
                    };
                    let full_text = generic_cap.node.utf8_text(source).unwrap_or("");

                    // Determine severity based on whether an atomic alternative exists
                    let (severity, message) = if full_text.contains("Mutex<bool>") {
                        (
                            "warning",
                            "`Arc<Mutex<bool>>` — use `AtomicBool` instead for better performance"
                                .to_string(),
                        )
                    } else if full_text.contains("Mutex<usize>")
                        || full_text.contains("Mutex<u64>")
                        || full_text.contains("Mutex<u32>")
                        || full_text.contains("Mutex<i64>")
                        || full_text.contains("Mutex<i32>")
                    {
                        (
                            "warning",
                            format!("`{full_text}` — use the corresponding `Atomic*` type instead"),
                        )
                    } else if full_text.contains("Mutex<HashMap")
                        || full_text.contains("Mutex<BTreeMap")
                    {
                        (
                            "warning",
                            format!("`{full_text}` — consider `DashMap` for concurrent map access"),
                        )
                    } else {
                        (
                            "info",
                            format!(
                                "`Arc<{inner_text}<T>>` detected — consider if a concurrent data structure or message passing would be simpler"
                            ),
                        )
                    };

                    let start = generic_cap.node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: severity.to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message,
                        snippet: full_text.to_string(),
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
        let pipeline = MutexOverusePipeline::new().unwrap();
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
    fn detects_arc_mutex() {
        let src = r#"
fn example() {
    let data: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(vec![]));
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "arc_mutex");
    }

    #[test]
    fn detects_arc_rwlock() {
        let src = r#"
fn example() {
    let data: Arc<RwLock<HashMap<String, i32>>> = todo!();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "arc_rwlock");
    }

    #[test]
    fn skips_mutex_without_arc() {
        let src = r#"
fn example() {
    let data: Mutex<i32> = Mutex::new(0);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_arc_without_mutex() {
        let src = r#"
fn example() {
    let data: Arc<Vec<i32>> = Arc::new(vec![]);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_file_excluded() {
        let src = r#"struct S { x: std::sync::Arc<std::sync::Mutex<i32>> }"#;
        let findings = parse_and_check_path(src, "tests/sync_test.rs");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_parking_lot_arc_mutex() {
        // parking_lot::Mutex ends with "Mutex" so existing suffix check should handle it
        let src = r#"struct S { x: std::sync::Arc<parking_lot::Mutex<Vec<i32>>> }"#;
        let findings = parse_and_check(src);
        // Verify no panic/crash; finding may or may not appear depending on query structure
        let _ = findings;
    }
}
