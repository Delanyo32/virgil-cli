use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{Pipeline, PipelineContext};
use crate::graph::CodeGraph;

use super::primitives::{compile_function_def_query, find_capture_index, node_text};

const SKIP_PARAMS: &[&str] = &["self", "cls"];
const SPLAT_KINDS: &[&str] = &["list_splat_pattern", "dictionary_splat_pattern"];

pub struct MissingTypeHintsPipeline {
    fn_query: Arc<Query>,
}

impl MissingTypeHintsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_def_query()?,
        })
    }
}

impl MissingTypeHintsPipeline {
    /// Determine whether a finding is for a function that is part of the cross-module
    /// public API (called from another file) or lives in an inherently API-facing file.
    fn is_cross_module_api(
        &self,
        finding: &AuditFinding,
        file_path: &str,
        graph: &CodeGraph,
    ) -> bool {
        // Files that are inherently API-facing
        let api_patterns = [
            "__init__.py",
            "/api/",
            "/views/",
            "/routes/",
            "/endpoints/",
        ];
        if api_patterns.iter().any(|p| file_path.contains(p)) {
            return true;
        }

        // finding.line is 1-indexed, graph stores 0-indexed start_line
        let start_line = finding.line - 1;

        // Look up the function in the graph
        let Some(sym_idx) = graph.find_symbol(file_path, start_line) else {
            // Function not in graph — it has no known callers, suppress
            return false;
        };

        // Check if the function has callers from other files
        let callers = graph.traverse_callers(&[sym_idx], 1);

        for caller_idx in &callers {
            match &graph.graph[*caller_idx] {
                crate::graph::NodeWeight::CallSite {
                    file_path: caller_file,
                    ..
                } => {
                    if caller_file != file_path {
                        return true; // Called from another file — cross-module API
                    }
                }
                crate::graph::NodeWeight::Symbol {
                    file_path: caller_file,
                    ..
                } => {
                    if caller_file != file_path {
                        return true;
                    }
                }
                _ => {}
            }
        }

        // No cross-module callers found — suppress
        false
    }
}

impl Pipeline for MissingTypeHintsPipeline {
    fn name(&self) -> &str {
        "missing_type_hints"
    }

    fn description(&self) -> &str {
        "Detects public functions missing parameter or return type annotations"
    }

    fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
        let base = self.check(ctx.tree, ctx.source, ctx.file_path);

        // When graph is available, only keep findings for cross-module API functions
        if let Some(graph) = ctx.graph {
            return base
                .into_iter()
                .filter(|f| self.is_cross_module_api(f, ctx.file_path, graph))
                .collect();
        }

        base
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");
        let params_idx = find_capture_index(&self.fn_query, "params");
        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let params_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == params_idx)
                .map(|c| c.node);
            let def_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_def_idx)
                .map(|c| c.node);

            if let (Some(name_node), Some(params_node), Some(def_node)) =
                (name_node, params_node, def_node)
            {
                let fn_name = node_text(name_node, source);

                // Skip private functions
                if fn_name.starts_with('_') {
                    continue;
                }

                let start = def_node.start_position();

                // Check for missing return type
                let has_return_type = def_node.child_by_field_name("return_type").is_some();
                if !has_return_type {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "missing_return_type".to_string(),
                        message: format!(
                            "function `{fn_name}` is missing a return type annotation"
                        ),
                        snippet: format!("def {fn_name}(...)"),
                    });
                }

                // Check for untyped parameters
                let untyped_params: Vec<String> = (0..params_node.named_child_count())
                    .filter_map(|i| params_node.named_child(i))
                    .filter(|child| {
                        // identifier = untyped param, default_parameter = untyped with default
                        child.kind() == "identifier" || child.kind() == "default_parameter"
                    })
                    .filter(|child| {
                        let name = if child.kind() == "identifier" {
                            node_text(*child, source)
                        } else {
                            // default_parameter: get the name child
                            child
                                .child_by_field_name("name")
                                .map(|n| node_text(n, source))
                                .unwrap_or("")
                        };
                        !SKIP_PARAMS.contains(&name)
                    })
                    .filter(|child| !SPLAT_KINDS.contains(&child.kind()))
                    .map(|child| {
                        if child.kind() == "identifier" {
                            node_text(child, source).to_string()
                        } else {
                            child
                                .child_by_field_name("name")
                                .map(|n| node_text(n, source).to_string())
                                .unwrap_or_default()
                        }
                    })
                    .collect();

                if !untyped_params.is_empty() {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "missing_param_type".to_string(),
                        message: format!(
                            "function `{fn_name}` has untyped parameters: {}",
                            untyped_params.join(", ")
                        ),
                        snippet: format!("def {fn_name}(...)"),
                    });
                }
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingTypeHintsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    fn parse_and_check_with_context(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingTypeHintsPipeline::new().unwrap();
        let id_counts = HashMap::new();
        let ctx = PipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: None,
        };
        pipeline.check_with_context(&ctx)
    }

    fn parse_and_check_with_graph(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MissingTypeHintsPipeline::new().unwrap();
        let id_counts = HashMap::new();
        // Create an empty graph — no callers for any symbol
        let graph = crate::graph::CodeGraph::new();
        let ctx = PipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: Some(&graph),
        };
        pipeline.check_with_context(&ctx)
    }

    #[test]
    fn detects_missing_param_and_return() {
        let src = "def foo(x, y):\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "missing_return_type"));
        assert!(findings.iter().any(|f| f.pattern == "missing_param_type"));
    }

    #[test]
    fn clean_fully_typed() {
        let src = "def foo(x: int, y: str) -> bool:\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_private_function() {
        let src = "def _internal(x):\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_self_param() {
        let src = "class Foo:\n    def bar(self, x: int) -> None:\n        pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_missing_return_only() {
        let src = "def foo(x: int):\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_return_type");
    }

    #[test]
    fn context_without_graph_returns_all_findings() {
        // Without a graph, check_with_context should return the same results as check()
        let src = "def foo(x, y):\n    pass\n";
        let findings = parse_and_check_with_context(src);
        assert!(findings.iter().any(|f| f.pattern == "missing_return_type"));
        assert!(findings.iter().any(|f| f.pattern == "missing_param_type"));
    }

    #[test]
    fn context_with_empty_graph_suppresses_non_api_findings() {
        // Empty graph = no callers = no cross-module API = suppress
        let src = "def foo(x, y):\n    pass\n";
        let findings = parse_and_check_with_graph(src, "mymodule.py");
        assert!(
            findings.is_empty(),
            "should suppress when no cross-module callers"
        );
    }

    #[test]
    fn context_with_graph_keeps_init_py_findings() {
        // __init__.py is always API-facing
        let src = "def foo(x, y):\n    pass\n";
        let findings = parse_and_check_with_graph(src, "package/__init__.py");
        assert!(
            !findings.is_empty(),
            "should keep findings for __init__.py"
        );
    }
}
