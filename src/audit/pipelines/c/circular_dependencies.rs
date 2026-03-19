use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

const HUB_MODULE_THRESHOLD: usize = 5;

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

pub struct CircularDependenciesPipeline {
    include_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new() -> Result<Self> {
        let include_query_str = r#"
(preproc_include
  path: (string_literal) @include_path) @include_dir
"#;
        let include_query = Query::new(&c_lang(), include_query_str)
            .with_context(|| "failed to compile include query for C circular deps")?;

        Ok(Self {
            include_query: Arc::new(include_query),
        })
    }

    /// Extract internal include targets (only `#include "..."`, not `#include <...>`).
    fn extract_internal_includes(&self, source: &[u8], tree: &Tree) -> Vec<(String, u32, u32)> {
        let mut cursor = QueryCursor::new();
        let path_idx = find_capture_index(&self.include_query, "include_path");
        let mut matches = cursor.matches(&self.include_query, tree.root_node(), source);

        let mut targets = Vec::new();
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == path_idx {
                    let text = node_text(cap.node, source);
                    // string_literal includes quotes: "header.h"
                    // Strip the surrounding quotes to get the raw path
                    let path = text.trim_matches('"');
                    if !path.is_empty() {
                        let pos = cap.node.start_position();
                        targets.push((path.to_string(), pos.row as u32 + 1, pos.column as u32 + 1));
                    }
                }
            }
        }
        targets
    }
}

impl Pipeline for CircularDependenciesPipeline {
    fn name(&self) -> &str {
        "circular_dependencies"
    }

    fn description(&self) -> &str {
        "Detects high fan-out internal includes that indicate circular dependency risk"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let targets = self.extract_internal_includes(source, tree);

        if targets.is_empty() {
            return findings;
        }

        // Count distinct internal include paths
        let distinct_includes: HashSet<&str> = targets.iter().map(|(t, _, _)| t.as_str()).collect();
        let fan_out = distinct_includes.len();

        // Pattern: hub_module_bidirectional
        // Flag files with high fan-out of internal includes
        if fan_out >= HUB_MODULE_THRESHOLD {
            let include_list: Vec<&str> = distinct_includes.into_iter().collect();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "circular_dependencies".to_string(),
                pattern: "hub_module_bidirectional".to_string(),
                message: format!(
                    "File includes {} distinct internal headers (threshold: {}): {}",
                    fan_out,
                    HUB_MODULE_THRESHOLD,
                    include_list.join(", ")
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"
#include "auth.h"
#include "billing.h"
#include "cache.h"
#include "config.h"
#include "database.h"
#include "logging.h"
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "hub_module_bidirectional")
        );
    }

    #[test]
    fn no_hub_for_few_includes() {
        let src = r#"
#include "config.h"
#include "database.h"
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "hub_module_bidirectional")
        );
    }

    #[test]
    fn ignores_system_includes() {
        let src = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>
#include <errno.h>
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn counts_only_distinct_includes() {
        let src = r#"
#include "config.h"
#include "config.h"
#include "config.h"
#include "config.h"
#include "config.h"
"#;
        let findings = parse_and_check(src);
        // Only 1 distinct include, below threshold
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "hub_module_bidirectional")
        );
    }
}
