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

fn php_lang() -> tree_sitter::Language {
    Language::Php.tree_sitter_language()
}

pub struct CircularDependenciesPipeline {
    use_query: Arc<Query>,
    include_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new() -> Result<Self> {
        // Match namespace use declarations (e.g., use App\Models\User;)
        let use_query_str = r#"
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name) @import_path)) @use_decl
"#;
        let use_query = Query::new(&php_lang(), use_query_str)
            .with_context(|| "failed to compile namespace use query for PHP circular deps")?;

        // Match require/include expressions
        let include_query_str = r#"
[
  (expression_statement (include_expression (_) @path)) @include_stmt
  (expression_statement (include_once_expression (_) @path)) @include_stmt
  (expression_statement (require_expression (_) @path)) @include_stmt
  (expression_statement (require_once_expression (_) @path)) @include_stmt
]
"#;
        let include_query = Query::new(&php_lang(), include_query_str)
            .with_context(|| "failed to compile include/require query for PHP circular deps")?;

        Ok(Self {
            use_query: Arc::new(use_query),
            include_query: Arc::new(include_query),
        })
    }
}

impl Pipeline for CircularDependenciesPipeline {
    fn name(&self) -> &str {
        "circular_dependencies"
    }

    fn description(&self) -> &str {
        "Detects high fan-out imports that indicate circular dependency risk"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();
        let mut distinct_imports: HashSet<String> = HashSet::new();

        // Count distinct namespace use declarations
        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.use_query, "import_path");
            let mut matches = cursor.matches(&self.use_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        let text = node_text(cap.node, source);
                        distinct_imports.insert(text.to_string());
                    }
                }
            }
        }

        // Count distinct require/include paths (only internal ones starting with '.')
        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.include_query, "path");
            let mut matches = cursor.matches(&self.include_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        let text = node_text(cap.node, source);
                        // Strip quotes from string literals
                        let clean = text.trim_matches('\'').trim_matches('"');
                        if clean.starts_with('.') {
                            distinct_imports.insert(clean.to_string());
                        }
                    }
                }
            }
        }

        let fan_out = distinct_imports.len();

        // Pattern: hub_module_bidirectional
        if fan_out >= HUB_MODULE_THRESHOLD {
            let import_list: Vec<&str> = distinct_imports.iter().map(|s| s.as_str()).collect();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "circular_dependencies".to_string(),
                pattern: "hub_module_bidirectional".to_string(),
                message: format!(
                    "Module imports from {} distinct sources (threshold: {}): {}",
                    fan_out,
                    HUB_MODULE_THRESHOLD,
                    import_list.join(", ")
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
        parser.set_language(&php_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"<?php
use App\Auth\AuthManager;
use App\Billing\PaymentGateway;
use App\Cache\CacheStore;
use App\Config\ConfigLoader;
use App\Database\ConnectionPool;
use App\Logging\Logger;
use App\Queue\JobDispatcher;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn no_hub_for_few_imports() {
        let src = r#"<?php
use App\Config\ConfigLoader;
use App\Database\ConnectionPool;
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn counts_include_require_internal() {
        let src = r#"<?php
use App\Auth\AuthManager;
use App\Billing\PaymentGateway;
use App\Cache\CacheStore;
require './helpers/functions.php';
include './helpers/constants.php';
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn ignores_external_includes() {
        let src = r#"<?php
require 'vendor/autoload.php';
include 'config.php';
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
