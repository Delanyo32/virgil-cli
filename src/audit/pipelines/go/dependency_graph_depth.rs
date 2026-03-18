use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_path_depth;
use crate::language::Language;
use super::primitives::{extract_snippet, find_capture_index, node_text};

const DEEP_IMPORT_DEPTH_THRESHOLD: usize = 4;

fn go_lang() -> tree_sitter::Language {
    Language::Go.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    import_path_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        let import_path_query_str = r#"
(import_spec
  path: (interpreted_string_literal) @import_path) @import_spec
"#;
        let import_path_query = Query::new(&go_lang(), import_path_query_str)
            .with_context(|| "failed to compile import path query for Go dependency depth")?;

        Ok(Self {
            import_path_query: Arc::new(import_path_query),
        })
    }
}

impl Pipeline for DependencyGraphDepthPipeline {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detects deeply nested import paths in Go files"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Pattern: deep_import_chain - check path depth of import paths
        // Go doesn't have barrel file re-exports, so we only check deep imports
        let mut cursor = QueryCursor::new();
        let path_idx = find_capture_index(&self.import_path_query, "import_path");
        let mut matches = cursor.matches(&self.import_path_query, root, source);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == path_idx {
                    let raw = node_text(cap.node, source);
                    // Strip surrounding quotes from interpreted_string_literal
                    let path = raw.trim_matches('"');
                    let depth = count_path_depth(path, "/");
                    if depth >= DEEP_IMPORT_DEPTH_THRESHOLD {
                        let pos = cap.node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: pos.row as u32 + 1,
                            column: pos.column as u32 + 1,
                            severity: "info".to_string(),
                            pipeline: "dependency_graph_depth".to_string(),
                            pattern: "deep_import_chain".to_string(),
                            message: format!(
                                "Import path has {} levels of nesting (threshold: {}): {}",
                                depth, DEEP_IMPORT_DEPTH_THRESHOLD, path
                            ),
                            snippet: extract_snippet(source, cap.node, 1),
                        });
                    }
                }
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"package main

import (
    "github.com/myorg/myapp/internal/platform/database/postgres/migrations"
)

func main() {}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"package main

import (
    "fmt"
    "net/http"
    "myapp/config"
)

func main() {}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_findings_for_no_imports() {
        let src = r#"package main

func main() {}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_deep_imports() {
        let src = r#"package handler

import (
    "github.com/myorg/myapp/internal/platform/database/postgres"
    "github.com/myorg/myapp/internal/domain/orders/aggregates/events"
)

func Handle() {}
"#;
        let findings = parse_and_check(src);
        let deep_findings: Vec<_> = findings.iter().filter(|f| f.pattern == "deep_import_chain").collect();
        assert!(deep_findings.len() >= 2);
    }
}
