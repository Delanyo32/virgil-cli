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

fn python_lang() -> tree_sitter::Language {
    Language::Python.tree_sitter_language()
}

pub struct CircularDependenciesPipeline {
    import_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new() -> Result<Self> {
        // Match import_from_statement with relative_import (internal) and
        // import_from_statement with dotted_name, plus plain import_statement.
        let import_query_str = r#"
[
  (import_from_statement
    module_name: (relative_import) @source) @import_stmt
  (import_from_statement
    module_name: (dotted_name) @source) @import_stmt
  (import_statement
    name: (dotted_name) @source) @import_stmt
]
"#;
        let import_query = Query::new(&python_lang(), import_query_str)
            .with_context(|| "failed to compile import query for Python circular deps")?;

        Ok(Self {
            import_query: Arc::new(import_query),
        })
    }

    fn extract_internal_import_sources(
        &self,
        source: &[u8],
        tree: &Tree,
    ) -> Vec<(String, u32, u32)> {
        let mut cursor = QueryCursor::new();
        let source_idx = find_capture_index(&self.import_query, "source");
        let mut matches = cursor.matches(&self.import_query, tree.root_node(), source);

        let mut targets = Vec::new();
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == source_idx {
                    let text = node_text(cap.node, source);
                    // Internal imports in Python: relative imports starting with "."
                    // relative_import nodes contain the dot prefix
                    if cap.node.kind() == "relative_import" || text.starts_with('.') {
                        let module_target = extract_module_target(text);
                        let pos = cap.node.start_position();
                        targets.push((module_target, pos.row as u32 + 1, pos.column as u32 + 1));
                    }
                }
            }
        }
        targets
    }
}

/// Extract the target module from a Python import path.
/// e.g., ".models.user" -> ".models"
/// e.g., "..utils" -> "..utils"
/// e.g., ".sub" -> ".sub"
fn extract_module_target(path: &str) -> String {
    // For relative imports, keep the full dotted path as the module target
    // since Python relative imports identify modules by their dot-prefixed path
    path.to_string()
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
        let targets = self.extract_internal_import_sources(source, tree);

        if targets.is_empty() {
            return findings;
        }

        // Count distinct import sources for fan-out
        let distinct_modules: HashSet<&str> = targets.iter().map(|(t, _, _)| t.as_str()).collect();
        let fan_out = distinct_modules.len();

        // Pattern: hub_module_bidirectional - high fan-out from internal imports
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&python_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"
from .auth import AuthManager
from .billing import PaymentProcessor
from .cache import CacheLayer
from .config import AppConfig
from .database import Pool
from .logging import Logger
from .messaging import EventBus
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "hub_module_bidirectional")
        );
    }

    #[test]
    fn no_hub_for_few_imports() {
        let src = r#"
from .config import AppConfig
from .database import Pool
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "hub_module_bidirectional")
        );
    }

    #[test]
    fn ignores_external_imports() {
        let src = r#"
import os
import json
import hashlib
from datetime import datetime
from collections import defaultdict
from typing import Optional
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn counts_relative_imports_with_multiple_dots() {
        let src = r#"
from .auth import AuthManager
from ..billing import PaymentProcessor
from .cache import CacheLayer
from ...config import AppConfig
from .database import Pool
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "hub_module_bidirectional")
        );
    }
}
