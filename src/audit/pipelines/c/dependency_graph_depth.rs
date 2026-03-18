use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_path_depth;
use crate::language::Language;
use super::primitives::{extract_snippet, find_capture_index, node_text};

const BARREL_REEXPORT_THRESHOLD: usize = 5;
const DEEP_IMPORT_DEPTH_THRESHOLD: usize = 4;

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    internal_include_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        let internal_include_query_str = r#"
(preproc_include
  path: (string_literal) @include_path) @include_dir
"#;
        let internal_include_query = Query::new(&c_lang(), internal_include_query_str)
            .with_context(|| "failed to compile include query for C dependency depth")?;

        Ok(Self {
            internal_include_query: Arc::new(internal_include_query),
        })
    }
}

impl Pipeline for DependencyGraphDepthPipeline {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detects umbrella header re-exports and deeply nested include paths"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let mut internal_include_count = 0usize;

        // Scan internal includes (#include "...")
        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.internal_include_query, "include_path");
            let mut matches = cursor.matches(&self.internal_include_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        let text = node_text(cap.node, source);
                        let path = text.trim_matches('"');

                        // Count internal includes for barrel detection
                        internal_include_count += 1;

                        // Pattern 2: deep_import_chain - check path depth
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
                                    "Include path has {} levels of nesting (threshold: {}): {}",
                                    depth, DEEP_IMPORT_DEPTH_THRESHOLD, path
                                ),
                                snippet: extract_snippet(source, cap.node, 1),
                            });
                        }
                    }
                }
            }
        }

        // Pattern 1: barrel_file_reexport - header files with many internal includes
        if file_path.ends_with(".h") && internal_include_count >= BARREL_REEXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "dependency_graph_depth".to_string(),
                pattern: "barrel_file_reexport".to_string(),
                message: format!(
                    "Header file has {} internal #include directives (threshold: {})",
                    internal_include_count, BARREL_REEXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_barrel_header() {
        let src = r#"
#ifndef PROJECT_H
#define PROJECT_H
#include "config.h"
#include "logging.h"
#include "memory.h"
#include "networking.h"
#include "parsing.h"
#endif
"#;
        let findings = parse_and_check(src, "project.h");
        assert!(findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_c_files() {
        let src = r#"
#include "config.h"
#include "logging.h"
#include "memory.h"
#include "networking.h"
#include "parsing.h"
void init(void) {}
"#;
        let findings = parse_and_check(src, "main.c");
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_few_includes() {
        let src = r#"
#ifndef UTILS_H
#define UTILS_H
#include "config.h"
#include "logging.h"
#endif
"#;
        let findings = parse_and_check(src, "utils.h");
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"
#include "platform/drivers/gpu/vulkan/pipeline.h"
"#;
        let findings = parse_and_check(src, "renderer.c");
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"
#include "config.h"
#include "utils/helpers.h"
"#;
        let findings = parse_and_check(src, "main.c");
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }
}
