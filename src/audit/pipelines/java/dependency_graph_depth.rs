use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_path_depth;
use crate::language::Language;
use super::primitives::{extract_snippet, find_capture_index, node_text};

const DEEP_IMPORT_DEPTH_THRESHOLD: usize = 4;

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    import_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        // Capture entire import_declaration to handle both regular and wildcard imports
        let import_query_str = r#"
(import_declaration) @import_decl
"#;
        let import_query = Query::new(&java_lang(), import_query_str)
            .with_context(|| "failed to compile import path query for Java dependency depth")?;

        Ok(Self {
            import_query: Arc::new(import_query),
        })
    }

    /// Parse the import path from an import_declaration node text.
    fn parse_import_path(text: &str) -> Option<String> {
        let text = text.trim();
        let text = text.strip_prefix("import")?.trim();
        let text = text.strip_prefix("static").unwrap_or(text).trim();
        let text = text.strip_suffix(';').unwrap_or(text).trim();
        if text.is_empty() {
            return None;
        }
        Some(text.to_string())
    }
}

impl Pipeline for DependencyGraphDepthPipeline {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detects deeply nested import paths indicating excessive architectural layering"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Pattern: deep_import_chain
        // Count dot-separated segments in import paths. Flag >= 4 segments
        // beyond the conventional prefix (which we don't strip — we count raw segments).
        let mut cursor = QueryCursor::new();
        let decl_idx = find_capture_index(&self.import_query, "import_decl");
        let mut matches = cursor.matches(&self.import_query, root, source);

        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == decl_idx {
                    let text = node_text(cap.node, source);
                    if let Some(path) = Self::parse_import_path(text) {
                        // Strip wildcard suffix if present
                        let clean = path.trim_end_matches(".*");
                        let depth = count_path_depth(clean, ".");
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
                                    depth, DEEP_IMPORT_DEPTH_THRESHOLD, path
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
        parser.set_language(&java_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"
import com.myapp.infrastructure.persistence.repositories.OrderRepository;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"
import com.myapp.Config;
import java.util.List;
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn detects_deep_wildcard_import() {
        let src = r#"
import com.myapp.domain.aggregates.orders.valueobjects.*;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn threshold_boundary() {
        // Exactly 4 segments: com.myapp.services.OrderService -> depth = 4
        let src = "import com.myapp.services.OrderService;\n";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn below_threshold() {
        // 3 segments: java.util.List -> depth = 3
        let src = "import java.util.List;\n";
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }
}
