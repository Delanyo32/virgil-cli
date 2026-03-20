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

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    reexport_query: Arc<Query>,
    import_path_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        // Barrel re-export: export_statement with a source string (re-exports like `export { X } from './mod'`)
        let reexport_query_str = r#"
(export_statement
  source: (string) @source) @reexport
"#;
        let reexport_query = Query::new(&js_lang(), reexport_query_str)
            .with_context(|| "failed to compile re-export query for JavaScript dependency depth")?;

        // Import path: import_statement with a source string
        let import_path_query_str = r#"
(import_statement
  source: (string) @import_path) @import_stmt
"#;
        let import_path_query = Query::new(&js_lang(), import_path_query_str).with_context(
            || "failed to compile import path query for JavaScript dependency depth",
        )?;

        Ok(Self {
            reexport_query: Arc::new(reexport_query),
            import_path_query: Arc::new(import_path_query),
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

        // Pattern 1: barrel_file_reexport - count export statements with a source (re-exports)
        let mut reexport_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let reexport_idx = find_capture_index(&self.reexport_query, "reexport");
            let mut matches = cursor.matches(&self.reexport_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == reexport_idx
                        && (cap.node.parent().is_some_and(|p| p.kind() == "program")
                            || cap.node.parent().is_none())
                        {
                            reexport_count += 1;
                        }
                }
            }
        }

        if reexport_count >= BARREL_REEXPORT_THRESHOLD {
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "dependency_graph_depth".to_string(),
                pattern: "barrel_file_reexport".to_string(),
                message: format!(
                    "File has {} re-export statements (threshold: {})",
                    reexport_count, BARREL_REEXPORT_THRESHOLD
                ),
                snippet: String::new(),
            });
        }

        // Pattern 2: deep_import_chain - count ../ segments in relative import paths
        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.import_path_query, "import_path");
            let mut matches = cursor.matches(&self.import_path_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        let raw = node_text(cap.node, source);
                        // Strip surrounding quotes
                        let path = raw.trim_matches(|c| c == '\'' || c == '"');
                        // Only check relative imports
                        if path.starts_with("./") || path.starts_with("../") {
                            let depth = count_path_depth(path, "/");
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
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "index.js")
    }

    #[test]
    fn detects_barrel_reexports() {
        let src = r#"
export { Button } from './Button';
export { TextField } from './TextField';
export { Select } from './Select';
export { Checkbox } from './Checkbox';
export { RadioGroup } from './RadioGroup';
export { DatePicker } from './DatePicker';
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn no_barrel_for_few_reexports() {
        let src = r#"
export { Button } from './Button';
export { TextField } from './TextField';
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"
import { validateOrder } from '../../../../domain/orders/validation/rules';
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"
import { foo } from './foo';
import { bar } from '../bar';
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn ignores_external_imports_for_depth() {
        let src = r#"
import express from 'express';
import lodash from 'lodash';
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }
}
