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

fn go_lang() -> tree_sitter::Language {
    Language::Go.tree_sitter_language()
}

pub struct CircularDependenciesPipeline {
    import_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new() -> Result<Self> {
        let import_query_str = r#"
(import_spec
  path: (interpreted_string_literal) @import_path) @import_spec
"#;
        let import_query = Query::new(&go_lang(), import_query_str)
            .with_context(|| "failed to compile import query for Go circular deps")?;

        Ok(Self {
            import_query: Arc::new(import_query),
        })
    }

    fn extract_import_paths(&self, source: &[u8], tree: &Tree) -> Vec<(String, u32, u32)> {
        let mut cursor = QueryCursor::new();
        let path_idx = find_capture_index(&self.import_query, "import_path");
        let mut matches = cursor.matches(&self.import_query, tree.root_node(), source);

        let mut imports = Vec::new();
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == path_idx {
                    let raw = node_text(cap.node, source);
                    // Strip surrounding quotes from interpreted_string_literal
                    let path = raw.trim_matches('"');
                    let pos = cap.node.start_position();
                    imports.push((path.to_string(), pos.row as u32 + 1, pos.column as u32 + 1));
                }
            }
        }
        imports
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
        let imports = self.extract_import_paths(source, tree);

        if imports.is_empty() {
            return findings;
        }

        // Since Go doesn't have syntactic markers for intra-project imports,
        // count ALL distinct import paths for fan-out estimation.
        let distinct_paths: HashSet<&str> = imports.iter().map(|(p, _, _)| p.as_str()).collect();
        let fan_out = distinct_paths.len();

        // Pattern: hub_module_bidirectional
        // Flag files with >= 5 distinct import paths as potential hub modules
        if fan_out >= HUB_MODULE_THRESHOLD {
            let path_list: Vec<&str> = distinct_paths.into_iter().collect();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "circular_dependencies".to_string(),
                pattern: "hub_module_bidirectional".to_string(),
                message: format!(
                    "Module imports from {} distinct packages (threshold: {}): {}",
                    fan_out,
                    HUB_MODULE_THRESHOLD,
                    path_list.join(", ")
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
        parser.set_language(&go_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.go")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"package main

import (
    "myapp/pkg/auth"
    "myapp/pkg/billing"
    "myapp/pkg/cache"
    "myapp/pkg/config"
    "myapp/pkg/db"
    "myapp/pkg/logging"
    "myapp/pkg/messaging"
)

func Init() {}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn no_hub_for_few_imports() {
        let src = r#"package main

import (
    "fmt"
    "os"
)

func main() {}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
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
    fn counts_distinct_imports_only() {
        // Even with grouped imports, only distinct paths count
        let src = r#"package main

import (
    "myapp/pkg/auth"
    "myapp/pkg/billing"
    "myapp/pkg/cache"
    "myapp/pkg/config"
    "fmt"
)

func main() {}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }
}
