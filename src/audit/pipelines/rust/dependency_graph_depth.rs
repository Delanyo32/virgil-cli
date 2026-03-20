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

fn rust_lang() -> tree_sitter::Language {
    Language::Rust.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    pub_use_query: Arc<Query>,
    use_path_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        let pub_use_query_str = r#"
(use_declaration
  (visibility_modifier) @vis
  argument: (_) @reexport_path) @pub_use
"#;
        let pub_use_query = Query::new(&rust_lang(), pub_use_query_str)
            .with_context(|| "failed to compile pub use query for Rust dependency depth")?;

        let use_path_query_str = r#"
(use_declaration
  argument: (_) @use_path) @use_decl
"#;
        let use_path_query = Query::new(&rust_lang(), use_path_query_str)
            .with_context(|| "failed to compile use path query for Rust dependency depth")?;

        Ok(Self {
            pub_use_query: Arc::new(pub_use_query),
            use_path_query: Arc::new(use_path_query),
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

        // Pattern 1: barrel_file_reexport - count pub use declarations
        let mut pub_use_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let pub_use_idx = find_capture_index(&self.pub_use_query, "pub_use");
            let mut matches = cursor.matches(&self.pub_use_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == pub_use_idx
                        && (cap.node.parent().is_some_and(|p| p.kind() == "source_file")
                            || cap.node.parent().is_none())
                    {
                        pub_use_count += 1;
                    }
                }
            }
        }

        if pub_use_count >= BARREL_REEXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "dependency_graph_depth".to_string(),
                pattern: "barrel_file_reexport".to_string(),
                message: format!(
                    "File has {} pub use re-exports (threshold: {})",
                    pub_use_count, BARREL_REEXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: deep_import_chain - check path depth of use declarations
        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.use_path_query, "use_path");
            let mut matches = cursor.matches(&self.use_path_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        let text = node_text(cap.node, source);
                        // Strip prefix (crate::, self::, super::) for depth counting
                        let clean = text
                            .strip_prefix("crate::")
                            .or_else(|| text.strip_prefix("self::"))
                            .or_else(|| text.strip_prefix("super::"))
                            .unwrap_or(text);
                        // Also strip any use-list syntax
                        let clean = clean
                            .split('{')
                            .next()
                            .unwrap_or(clean)
                            .trim_end_matches("::");
                        let depth = count_path_depth(clean, "::");
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&rust_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "mod.rs")
    }

    #[test]
    fn detects_barrel_reexports() {
        let src = r#"
pub use auth::{AuthService, TokenValidator};
pub use billing::{BillingService, InvoiceGenerator};
pub use email::{EmailService, TemplateRenderer};
pub use reporting::{ReportService, ChartBuilder};
pub use storage::{StorageService, FileManager};
pub use users::{UserService, ProfileManager};
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_few_reexports() {
        let src = r#"
pub use auth::AuthService;
pub use billing::BillingService;
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"
use crate::infrastructure::persistence::repositories::postgres::OrderRepository;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"
use crate::config::AppConfig;
use std::collections::HashMap;
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }
}
