use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_path_depth;
use crate::language::Language;

const BARREL_REEXPORT_THRESHOLD: usize = 5;
const DEEP_IMPORT_DEPTH_THRESHOLD: usize = 4;

fn cpp_lang() -> tree_sitter::Language {
    Language::Cpp.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    include_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        let include_query_str = r#"
(preproc_include
  path: (_) @include_path) @include_dir
"#;
        let include_query = Query::new(&cpp_lang(), include_query_str)
            .with_context(|| "failed to compile preproc_include query for C++ dependency depth")?;

        Ok(Self {
            include_query: Arc::new(include_query),
        })
    }
}

impl Pipeline for DependencyGraphDepthPipeline {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detects barrel file re-exports (umbrella headers) and deeply nested include paths"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let mut internal_include_count = 0usize;
        let mut deep_includes = Vec::new();

        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.include_query, "include_path");
            let dir_idx = find_capture_index(&self.include_query, "include_dir");
            let mut matches = cursor.matches(&self.include_query, root, source);
            while let Some(m) = matches.next() {
                let mut path_text = "";
                let mut path_node = None;
                let mut is_internal = false;
                let mut dir_node = None;

                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        path_text = node_text(cap.node, source);
                        path_node = Some(cap.node);
                        // Internal includes use string_literal: "header.h"
                        is_internal = cap.node.kind() == "string_literal";
                    }
                    if cap.index as usize == dir_idx {
                        dir_node = Some(cap.node);
                    }
                }

                if is_internal {
                    internal_include_count += 1;

                    // Check depth of include path (strip quotes)
                    let clean = path_text.trim_matches('"');
                    let depth = count_path_depth(clean, "/");
                    if depth >= DEEP_IMPORT_DEPTH_THRESHOLD
                        && let Some(pn) = path_node
                    {
                        let pos = pn.start_position();
                        let snippet = dir_node
                            .map(|n| extract_snippet(source, n, 1))
                            .unwrap_or_default();
                        deep_includes.push((
                            clean.to_string(),
                            depth,
                            pos.row as u32 + 1,
                            pos.column as u32 + 1,
                            snippet,
                        ));
                    }
                }
            }
        }

        // Pattern 1: barrel_file_reexport
        // A header with many internal includes acts as an umbrella/barrel header
        if internal_include_count >= BARREL_REEXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "dependency_graph_depth".to_string(),
                pattern: "barrel_file_reexport".to_string(),
                message: format!(
                    "File has {} internal #include directives (threshold: {})",
                    internal_include_count, BARREL_REEXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: deep_import_chain
        for (path, depth, line, col, snippet) in deep_includes {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line,
                column: col,
                severity: "info".to_string(),
                pipeline: "dependency_graph_depth".to_string(),
                pattern: "deep_import_chain".to_string(),
                message: format!(
                    "Include path has {} levels of nesting (threshold: {}): {}",
                    depth, DEEP_IMPORT_DEPTH_THRESHOLD, path
                ),
                snippet,
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&cpp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.hpp")
    }

    #[test]
    fn detects_barrel_header() {
        let src = r#"
#include "engine/audio.hpp"
#include "engine/input.hpp"
#include "engine/physics.hpp"
#include "engine/rendering.hpp"
#include "engine/networking.hpp"
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_few_includes() {
        let src = r#"
#include "config.hpp"
#include "utils.hpp"
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn system_includes_not_counted_as_barrel() {
        let src = r#"
#include <iostream>
#include <vector>
#include <string>
#include <map>
#include <algorithm>
#include <memory>
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"
#include "core/platform/graphics/vulkan/pipeline_state.hpp"
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"
#include "engine/audio.hpp"
#include "config.hpp"
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn deep_import_with_exact_threshold() {
        // 4 segments: a/b/c/d.hpp
        let src = r#"
#include "a/b/c/d.hpp"
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_for_three_segments() {
        // 3 segments: a/b/c.hpp
        let src = r#"
#include "a/b/c.hpp"
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }
}
