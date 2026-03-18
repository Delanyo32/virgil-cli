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

fn csharp_lang() -> tree_sitter::Language {
    Language::CSharp.tree_sitter_language()
}

pub struct CircularDependenciesPipeline {
    using_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new() -> Result<Self> {
        // C# using directives contain either a qualified_name or an identifier
        let using_query_str = r#"
(using_directive
  [
    (qualified_name) @using_path
    (identifier) @using_path
  ]) @using_decl
"#;
        let using_query = Query::new(&csharp_lang(), using_query_str)
            .with_context(|| "failed to compile using_directive query for C# circular deps")?;

        Ok(Self {
            using_query: Arc::new(using_query),
        })
    }
}

impl Pipeline for CircularDependenciesPipeline {
    fn name(&self) -> &str {
        "circular_dependencies"
    }

    fn description(&self) -> &str {
        "Detects high fan-out using directives that indicate circular dependency risk"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        // C# using directives are all treated as external (no syntactic way to
        // distinguish intra-project from external). We skip the mutual_import
        // pattern and only detect hub modules via fan-out.

        let mut distinct_namespaces: HashSet<String> = HashSet::new();
        let mut cursor = QueryCursor::new();
        let path_idx = find_capture_index(&self.using_query, "using_path");
        let mut matches = cursor.matches(&self.using_query, root, source);

        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == path_idx {
                    let text = node_text(cap.node, source);
                    distinct_namespaces.insert(text.to_string());
                }
            }
        }

        let fan_out = distinct_namespaces.len();

        // Pattern: hub_module_bidirectional
        if fan_out >= HUB_MODULE_THRESHOLD {
            let mut ns_list: Vec<&str> = distinct_namespaces.iter().map(|s| s.as_str()).collect();
            ns_list.sort();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "circular_dependencies".to_string(),
                pattern: "hub_module_bidirectional".to_string(),
                message: format!(
                    "Module imports from {} distinct namespaces (threshold: {}): {}",
                    fan_out,
                    HUB_MODULE_THRESHOLD,
                    ns_list.join(", ")
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
        parser.set_language(&csharp_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.cs")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"
using MyApp.Auth;
using MyApp.Billing;
using MyApp.Cache;
using MyApp.Config;
using MyApp.Database;
using MyApp.Logging;

namespace MyApp.Services
{
    public class ServiceLocator { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn no_hub_for_few_imports() {
        let src = r#"
using System;
using System.Linq;

namespace MyApp
{
    public class Foo { }
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn deduplicates_namespaces() {
        let src = r#"
using System;
using System;
using System.Linq;

namespace MyApp
{
    public class Foo { }
}
"#;
        let findings = parse_and_check(src);
        // Only 2 distinct namespaces, not 3
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn counts_qualified_and_simple_names() {
        let src = r#"
using System;
using MyApp.Auth;
using MyApp.Billing;
using MyApp.Cache;
using MyApp.Config;

namespace MyApp.Services
{
    public class Svc { }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }
}
