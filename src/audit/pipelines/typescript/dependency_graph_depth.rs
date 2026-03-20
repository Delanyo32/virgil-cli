use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

const BARREL_REEXPORT_THRESHOLD: usize = 5;
const DEEP_IMPORT_DEPTH_THRESHOLD: usize = 4;

pub struct DependencyGraphDepthPipeline {
    _language: Language,
    reexport_query: Arc<Query>,
    import_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();

        // Match export statements with a source (re-exports):
        // export { Foo } from './foo'
        // export type { Bar } from './bar'
        // export * from './baz'
        let reexport_query_str = r#"
(export_statement
  source: (string) @source) @reexport
"#;
        let reexport_query = Query::new(&ts_lang, reexport_query_str)
            .with_context(|| "failed to compile re-export query for TypeScript dependency depth")?;

        // Match import statements to check path depth
        let import_query_str = r#"
(import_statement
  source: (string) @import_path) @import_stmt
"#;
        let import_query = Query::new(&ts_lang, import_query_str)
            .with_context(|| "failed to compile import query for TypeScript dependency depth")?;

        Ok(Self {
            _language: language,
            reexport_query: Arc::new(reexport_query),
            import_query: Arc::new(import_query),
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

        // Pattern 1: barrel_file_reexport — count export statements with a source
        let mut reexport_count = 0usize;
        {
            let mut cursor = QueryCursor::new();
            let reexport_idx = find_capture_index(&self.reexport_query, "reexport");
            let mut matches = cursor.matches(&self.reexport_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == reexport_idx {
                        // Only count top-level re-exports
                        if cap.node.parent().is_none_or(|p| p.kind() == "program") {
                            reexport_count += 1;
                        }
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

        // Pattern 2: deep_import_chain — count ../ segments in import paths
        {
            let mut cursor = QueryCursor::new();
            let path_idx = find_capture_index(&self.import_query, "import_path");
            let mut matches = cursor.matches(&self.import_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == path_idx {
                        let raw = node_text(cap.node, source);
                        let path = raw.trim_matches(|c| c == '\'' || c == '"');
                        // Count ../ segments for relative imports
                        if path.starts_with("../") || path.starts_with("./") {
                            let depth = path.matches("../").count();
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
                                        "Import path has {} levels of parent traversal (threshold: {}): {}",
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

    fn ts_lang() -> tree_sitter::Language {
        Language::TypeScript.tree_sitter_language()
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_barrel_reexports() {
        let src = r#"
export { Button } from './Button';
export { TextField } from './TextField';
export { Select } from './Select';
export { Checkbox } from './Checkbox';
export { RadioGroup } from './RadioGroup';
export type { ButtonProps } from './Button';
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
import { AppConfig } from './config';
import { Pool } from '../database';
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn ignores_external_imports_for_depth() {
        let src = r#"
import React from 'react';
import { join } from 'path';
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn counts_type_reexports_in_barrel() {
        let src = r#"
export type { Foo } from './foo';
export type { Bar } from './bar';
export type { Baz } from './baz';
export type { Qux } from './qux';
export type { Quux } from './quux';
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }
}
