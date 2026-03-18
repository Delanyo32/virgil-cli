use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;
use super::primitives::{find_capture_index, node_text};

const HUB_MODULE_THRESHOLD: usize = 5;

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

pub struct CircularDependenciesPipeline {
    import_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new() -> Result<Self> {
        let import_query_str = r#"
(import_statement
  source: (string) @import_source) @import_stmt
"#;
        let import_query = Query::new(&js_lang(), import_query_str)
            .with_context(|| "failed to compile import statement query for JavaScript circular deps")?;

        Ok(Self {
            import_query: Arc::new(import_query),
        })
    }

    /// Extract relative import sources (starting with ./ or ../).
    /// Returns (source_path, line, column) tuples.
    fn extract_relative_imports(&self, source: &[u8], tree: &Tree) -> Vec<(String, u32, u32)> {
        let mut cursor = QueryCursor::new();
        let source_idx = find_capture_index(&self.import_query, "import_source");
        let mut matches = cursor.matches(&self.import_query, tree.root_node(), source);

        let mut targets = Vec::new();
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == source_idx {
                    let raw = node_text(cap.node, source);
                    // Strip surrounding quotes
                    let path = raw.trim_matches(|c| c == '\'' || c == '"');
                    // Only count relative imports
                    if path.starts_with("./") || path.starts_with("../") {
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
        "Detects high fan-out relative imports that indicate circular dependency risk"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let targets = self.extract_relative_imports(source, tree);

        if targets.is_empty() {
            return findings;
        }

        // Count distinct relative import sources for fan-out
        let distinct_sources: HashSet<&str> = targets.iter().map(|(t, _, _)| t.as_str()).collect();
        let fan_out = distinct_sources.len();

        // Pattern: hub_module_bidirectional
        if fan_out >= HUB_MODULE_THRESHOLD {
            let source_list: Vec<&str> = distinct_sources.into_iter().collect();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "circular_dependencies".to_string(),
                pattern: "hub_module_bidirectional".to_string(),
                message: format!(
                    "Module imports from {} distinct relative modules (threshold: {}): {}",
                    fan_out,
                    HUB_MODULE_THRESHOLD,
                    source_list.join(", ")
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
        parser.set_language(&js_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"
import { AuthManager } from './auth';
import { PaymentProcessor } from '../billing';
import { CacheLayer } from './cache';
import { AppConfig } from '../config';
import { Pool } from './database';
import { Logger } from '../logging';
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn no_hub_for_few_imports() {
        let src = r#"
import { AppConfig } from './config';
import { Pool } from './database';
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn ignores_external_imports() {
        let src = r#"
import React from 'react';
import express from 'express';
import lodash from 'lodash';
import axios from 'axios';
import chalk from 'chalk';
import dayjs from 'dayjs';
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn counts_relative_imports_only() {
        let src = r#"
import { foo } from './foo';
import { bar } from '../bar';
import { baz } from './baz';
import express from 'express';
import lodash from 'lodash';
"#;
        let findings = parse_and_check(src);
        // Only 3 relative imports, below threshold of 5
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }
}
