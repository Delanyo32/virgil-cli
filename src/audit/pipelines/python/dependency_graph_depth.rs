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

fn python_lang() -> tree_sitter::Language {
    Language::Python.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    relative_import_query: Arc<Query>,
    dotted_import_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        // Match relative imports: from .sub import ...
        let relative_import_query_str = r#"
(import_from_statement
  module_name: (relative_import) @rel_source) @rel_import
"#;
        let relative_import_query = Query::new(&python_lang(), relative_import_query_str)
            .with_context(
                || "failed to compile relative import query for Python dependency depth",
            )?;

        // Match absolute dotted imports: from package.sub.deep import ...
        // and plain import statements: import package.sub.deep
        let dotted_import_query_str = r#"
[
  (import_from_statement
    module_name: (dotted_name) @module_path) @import_stmt
  (import_statement
    name: (dotted_name) @module_path) @import_stmt
]
"#;
        let dotted_import_query = Query::new(&python_lang(), dotted_import_query_str)
            .with_context(|| "failed to compile dotted import query for Python dependency depth")?;

        Ok(Self {
            relative_import_query: Arc::new(relative_import_query),
            dotted_import_query: Arc::new(dotted_import_query),
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
        let is_init_file = file_path.ends_with("__init__.py");

        // Pattern 1: barrel_file_reexport - count `from .sub import ...` in __init__.py files
        {
            let mut relative_import_count = 0usize;
            let mut cursor = QueryCursor::new();
            let rel_import_idx = find_capture_index(&self.relative_import_query, "rel_import");
            let mut matches = cursor.matches(&self.relative_import_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == rel_import_idx {
                        // Only count top-level imports
                        if cap.node.parent().is_some_and(|p| p.kind() == "module") {
                            relative_import_count += 1;
                        }
                    }
                }
            }

            if relative_import_count >= BARREL_REEXPORT_THRESHOLD && is_init_file {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: 1,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: "dependency_graph_depth".to_string(),
                    pattern: "barrel_file_reexport".to_string(),
                    message: format!(
                        "File has {} relative re-export imports (threshold: {})",
                        relative_import_count, BARREL_REEXPORT_THRESHOLD
                    ),
                    snippet: String::new(),
                });
            }
        }

        // Pattern 2: deep_import_chain - check depth of dotted import paths
        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.dotted_import_query, "module_path");
            let mut matches = cursor.matches(&self.dotted_import_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        let text = node_text(cap.node, source);
                        let depth = count_path_depth(text, ".");
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
                                    depth, DEEP_IMPORT_DEPTH_THRESHOLD, text
                                ),
                                snippet: extract_snippet(source, cap.node, 1),
                            });
                        }
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

    fn parse_and_check(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&python_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_barrel_reexports_in_init() {
        let src = r#"
from .auth import AuthService, TokenValidator
from .billing import BillingService, InvoiceGenerator
from .email import EmailService, TemplateRenderer
from .reporting import ReportService, ChartBuilder
from .storage import StorageService, FileManager
from .users import UserService, ProfileManager
"#;
        let findings = parse_and_check(src, "__init__.py");
        assert!(findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_non_init_file() {
        let src = r#"
from .auth import AuthService
from .billing import BillingService
from .email import EmailService
from .reporting import ReportService
from .storage import StorageService
from .users import UserService
"#;
        let findings = parse_and_check(src, "services.py");
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_few_reexports() {
        let src = r#"
from .auth import AuthService
from .billing import BillingService
"#;
        let findings = parse_and_check(src, "__init__.py");
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"
from myapp.infrastructure.persistence.repositories.sqlalchemy import OrderRepository
"#;
        let findings = parse_and_check(src, "test.py");
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"
from myapp.config import AppConfig
import os.path
"#;
        let findings = parse_and_check(src, "test.py");
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn detects_deep_plain_import() {
        let src = r#"
import myapp.domain.aggregates.orders.value_objects
"#;
        let findings = parse_and_check(src, "test.py");
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }
}
