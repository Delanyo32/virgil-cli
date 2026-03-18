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

fn php_lang() -> tree_sitter::Language {
    Language::Php.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    use_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        // Match namespace use declarations with qualified name paths
        let use_query_str = r#"
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name) @use_path)) @use_decl
"#;
        let use_query = Query::new(&php_lang(), use_query_str)
            .with_context(|| "failed to compile namespace use query for PHP dependency depth")?;

        Ok(Self {
            use_query: Arc::new(use_query),
        })
    }
}

impl Pipeline for DependencyGraphDepthPipeline {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detects barrel file re-exports and deeply nested import paths"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let mut use_count = 0usize;
        let mut deep_imports: Vec<(String, u32, u32, tree_sitter::Node)> = Vec::new();

        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.use_query, "use_path");
            let mut matches = cursor.matches(&self.use_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        use_count += 1;
                        let text = node_text(cap.node, source);
                        // Count backslash-separated segments in the namespace path
                        let depth = count_path_depth(text, "\\");
                        if depth >= DEEP_IMPORT_DEPTH_THRESHOLD {
                            let pos = cap.node.start_position();
                            deep_imports.push((
                                text.to_string(),
                                pos.row as u32 + 1,
                                pos.column as u32 + 1,
                                cap.node,
                            ));
                        }
                    }
                }
            }
        }

        // Pattern 1: barrel_file_reexport - count namespace use declarations
        // In PHP, barrel files aggregate many use declarations from sub-namespaces
        if use_count >= BARREL_REEXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "dependency_graph_depth".to_string(),
                pattern: "barrel_file_reexport".to_string(),
                message: format!(
                    "File has {} namespace use declarations (threshold: {})",
                    use_count, BARREL_REEXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: deep_import_chain - check namespace path depth
        for (text, line, col, node) in &deep_imports {
            let depth = count_path_depth(text, "\\");
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: *line,
                column: *col,
                severity: "info".to_string(),
                pipeline: "dependency_graph_depth".to_string(),
                pattern: "deep_import_chain".to_string(),
                message: format!(
                    "Import path has {} levels of nesting (threshold: {}): {}",
                    depth, DEEP_IMPORT_DEPTH_THRESHOLD, text
                ),
                snippet: extract_snippet(source, *node, 1),
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
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_barrel_reexports() {
        let src = r#"<?php
use App\Services\Auth\AuthService;
use App\Services\Billing\BillingService;
use App\Services\Email\EmailService;
use App\Services\Reporting\ReportService;
use App\Services\Storage\StorageService;
use App\Services\Users\UserService;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_few_uses() {
        let src = r#"<?php
use App\Auth\AuthService;
use App\Billing\BillingService;
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"<?php
use App\Infrastructure\Persistence\Repositories\Contracts\IOrderRepository;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"<?php
use App\Models\User;
use App\Services\AuthService;
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }
}
