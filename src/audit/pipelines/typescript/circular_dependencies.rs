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

pub struct CircularDependenciesPipeline {
    _language: Language,
    import_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();

        let import_query_str = r#"
(import_statement
  source: (string) @source)
"#;
        let import_query = Query::new(&ts_lang, import_query_str)
            .with_context(|| "failed to compile import query for TypeScript circular deps")?;

        Ok(Self {
            _language: language,
            import_query: Arc::new(import_query),
        })
    }

    fn extract_internal_imports(&self, source: &[u8], tree: &Tree) -> Vec<(String, u32, u32)> {
        let mut cursor = QueryCursor::new();
        let source_idx = find_capture_index(&self.import_query, "source");
        let mut matches = cursor.matches(&self.import_query, tree.root_node(), source);

        let mut targets = Vec::new();
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == source_idx {
                    let raw = node_text(cap.node, source);
                    // Strip quotes from string literal
                    let path = raw.trim_matches(|c| c == '\'' || c == '"');
                    // Only count internal (relative) imports: ./ or ../
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
        "Detects high fan-out internal imports that indicate circular dependency risk"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let targets = self.extract_internal_imports(source, tree);

        if targets.is_empty() {
            return findings;
        }

        // Count distinct internal import targets
        let distinct_modules: HashSet<&str> = targets.iter().map(|(t, _, _)| t.as_str()).collect();
        let fan_out = distinct_modules.len();

        // Pattern: hub_module_bidirectional
        if fan_out >= HUB_MODULE_THRESHOLD {
            let module_list: Vec<&str> = distinct_modules.into_iter().collect();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "circular_dependencies".to_string(),
                pattern: "hub_module_bidirectional".to_string(),
                message: format!(
                    "Module imports from {} distinct internal modules (threshold: {}): {}",
                    fan_out,
                    HUB_MODULE_THRESHOLD,
                    module_list.join(", ")
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

    fn ts_lang() -> tree_sitter::Language {
        Language::TypeScript.tree_sitter_language()
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"
import { AuthManager } from './auth';
import { PaymentProcessor } from './billing';
import { CacheLayer } from './cache';
import { AppConfig } from './config';
import { Pool } from './database';
import { Logger } from './logging';
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
import { useState } from 'react';
import express from 'express';
import { join } from 'path';
import lodash from 'lodash';
import axios from 'axios';
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn counts_parent_relative_imports() {
        let src = r#"
import { AuthManager } from '../auth';
import { PaymentProcessor } from '../billing';
import { CacheLayer } from '../cache';
import { AppConfig } from '../config';
import { Pool } from '../database';
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }
}
