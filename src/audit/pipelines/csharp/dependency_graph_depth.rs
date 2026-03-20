use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{extract_snippet, find_capture_index, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines::helpers::count_path_depth;
use crate::language::Language;

const DEEP_IMPORT_DEPTH_THRESHOLD: usize = 4;

fn csharp_lang() -> tree_sitter::Language {
    Language::CSharp.tree_sitter_language()
}

pub struct DependencyGraphDepthPipeline {
    using_query: Arc<Query>,
}

impl DependencyGraphDepthPipeline {
    pub fn new() -> Result<Self> {
        let using_query_str = r#"
(using_directive
  [
    (qualified_name) @ns_path
    (identifier) @ns_path
  ]) @using_decl
"#;
        let using_query = Query::new(&csharp_lang(), using_query_str)
            .with_context(|| "failed to compile using_directive query for C# dependency depth")?;

        Ok(Self {
            using_query: Arc::new(using_query),
        })
    }
}

impl Pipeline for DependencyGraphDepthPipeline {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detects deeply nested namespace import paths in C# using directives"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // Pattern 1: barrel_file_reexport — N/A for C#. Skip entirely.

        // Pattern 2: deep_import_chain — count dot-separated segments in using paths.
        let mut cursor = QueryCursor::new();
        let path_idx = find_capture_index(&self.using_query, "ns_path");
        let mut matches = cursor.matches(&self.using_query, root, source);

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
                                "Using directive has {} namespace segments (threshold: {}): {}",
                                depth, DEEP_IMPORT_DEPTH_THRESHOLD, text
                            ),
                            snippet: extract_snippet(source, cap.node, 1),
                        });
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
        parser.set_language(&csharp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DependencyGraphDepthPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_deep_import_chain() {
        let src = r#"
using MyApp.Infrastructure.Persistence.Repositories.Abstractions;

namespace MyApp.Controllers
{
    public class OrderController { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn no_deep_import_for_shallow_path() {
        let src = r#"
using System;
using System.Linq;
using MyApp.Models;

namespace MyApp.Controllers
{
    public class OrderController { }
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "deep_import_chain"));
    }

    #[test]
    fn reports_correct_line() {
        let src = r#"using System;
using MyApp.Infrastructure.Persistence.Repositories.Abstractions;

namespace MyApp.Controllers
{
    public class Ctrl { }
}
"#;
        let findings = parse_and_check(src);
        let deep: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "deep_import_chain")
            .collect();
        assert_eq!(deep.len(), 1);
        assert_eq!(deep[0].line, 2);
    }

    #[test]
    fn no_barrel_reexport_findings() {
        // Barrel file re-export is N/A for C# — should never appear
        let src = r#"
using System;

namespace MyApp
{
    public class Foo { }
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "barrel_file_reexport"));
    }
}
