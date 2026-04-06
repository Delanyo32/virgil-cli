use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_noqa_suppressed;

use super::primitives::{compile_comparison_query, extract_snippet, find_capture_index, node_text};

const SUSPICIOUS_NAMES: &[&str] = &[
    "status", "kind", "type", "mode", "state", "action", "level", "category", "role", "variant",
    "phase", "stage",
];

pub struct StringlyTypedPipeline {
    comparison_query: Arc<Query>,
}

impl StringlyTypedPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            comparison_query: compile_comparison_query()?,
        })
    }

    fn is_suspicious_name(name: &str) -> bool {
        let lower = name.to_lowercase();
        SUSPICIOUS_NAMES.iter().any(|s| lower.contains(s))
    }

    /// Walk up the AST to find the enclosing function name (or "<module>" for top-level).
    fn enclosing_function_name(node: tree_sitter::Node, source: &[u8]) -> String {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_definition"
                && let Some(name_node) = parent.child_by_field_name("name")
            {
                return node_text(name_node, source).to_string();
            }
            current = parent.parent();
        }
        "<module>".to_string()
    }
}

impl GraphPipeline for StringlyTypedPipeline {
    fn name(&self) -> &str {
        "stringly_typed"
    }

    fn description(&self) -> &str {
        "Detects string comparisons on field names that should be enums"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.comparison_query, tree.root_node(), source);

        let comp_idx = find_capture_index(&self.comparison_query, "comparison");

        // (scope, variable_name) -> Vec<(line, column, snippet, string_value)>
        #[allow(clippy::type_complexity)]
        let mut var_comparisons: HashMap<(String, String), Vec<(u32, u32, String, String)>> =
            HashMap::new();

        while let Some(m) = matches.next() {
            let comp_cap = m.captures.iter().find(|c| c.index as usize == comp_idx);

            if let Some(comp_cap) = comp_cap {
                let node = comp_cap.node;

                if is_noqa_suppressed(source, node, self.name()) {
                    continue;
                }

                let scope = Self::enclosing_function_name(node, source);

                // Look for a string literal and an identifier/attribute among children
                let mut string_values: Vec<String> = Vec::new();
                let mut suspicious_identifier = None;
                let mut has_in_operator = false;

                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if !child.is_named() {
                            let op_text = node_text(child, source);
                            if op_text == "in" {
                                has_in_operator = true;
                            }
                            continue;
                        }
                        match child.kind() {
                            "string" => {
                                string_values.push(node_text(child, source).to_string());
                            }
                            "identifier" => {
                                let name = node_text(child, source);
                                if Self::is_suspicious_name(name) {
                                    suspicious_identifier = Some(name.to_string());
                                }
                            }
                            "attribute" => {
                                if let Some(attr) = child.child_by_field_name("attribute") {
                                    let name = node_text(attr, source);
                                    if Self::is_suspicious_name(name) {
                                        suspicious_identifier =
                                            Some(node_text(child, source).to_string());
                                    }
                                }
                            }
                            // Handle `in ["a", "b", "c"]` or `in ("a", "b", "c")`
                            "list" | "tuple" if has_in_operator => {
                                for j in 0..child.named_child_count() {
                                    if let Some(elem) = child.named_child(j)
                                        && elem.kind() == "string"
                                    {
                                        string_values
                                            .push(node_text(elem, source).to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if let Some(ident) = suspicious_identifier {
                    let start = node.start_position();
                    let snippet = extract_snippet(source, node, 1);
                    let key = (scope, ident);
                    for sv in string_values {
                        var_comparisons.entry(key.clone()).or_default().push((
                            start.row as u32 + 1,
                            start.column as u32 + 1,
                            snippet.clone(),
                            sv,
                        ));
                    }
                }
            }
        }

        // Only emit findings for variables with 3+ distinct string comparisons within same scope
        let mut findings = Vec::new();
        for ((_, ident), comparisons) in &var_comparisons {
            let unique_values: std::collections::HashSet<&String> =
                comparisons.iter().map(|(_, _, _, v)| v).collect();
            if unique_values.len() >= 3 {
                // Deduplicate findings by line (an `in` check produces multiple values on one line)
                let mut seen_lines = std::collections::HashSet::new();
                for (line, column, snippet, _) in comparisons {
                    if !seen_lines.insert(*line) {
                        continue;
                    }
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: *line,
                        column: *column,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "stringly_typed_comparison".to_string(),
                        message: format!(
                            "string comparison on `{ident}` — compared against {} distinct values, consider using an enum",
                            unique_values.len()
                        ),
                        snippet: snippet.clone(),
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
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = StringlyTypedPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_status_string_comparison() {
        let src = "if status == \"active\":\n    pass\nelif status == \"inactive\":\n    pass\nelif status == \"pending\":\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].pattern, "stringly_typed_comparison");
    }

    #[test]
    fn detects_attribute_comparison() {
        let src = "if obj.state == \"running\":\n    pass\nelif obj.state == \"stopped\":\n    pass\nelif obj.state == \"paused\":\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 3);
    }

    #[test]
    fn skips_few_comparisons() {
        // Only 1 comparison — should not trigger
        let src = "if status == \"active\":\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_two_comparisons() {
        // Only 2 distinct values — should not trigger (threshold is 3)
        let src = "if status == \"active\":\n    pass\nelif status == \"inactive\":\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_numeric_comparison() {
        let src = "if x == 5:\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_suspicious_name() {
        let src = "if name == \"alice\":\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_in_membership_test() {
        let src = "def process(status):\n    if status in [\"active\", \"inactive\", \"pending\"]:\n        pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1, "should detect in-membership with 3+ string values");
        assert_eq!(findings[0].pattern, "stringly_typed_comparison");
    }

    #[test]
    fn detects_in_tuple_membership() {
        let src = "def process(status):\n    if status in (\"active\", \"inactive\", \"pending\"):\n        pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn cross_function_no_grouping() {
        let src = "def foo():\n    if status == \"active\":\n        pass\n\ndef bar():\n    if status == \"inactive\":\n        pass\n\ndef baz():\n    if status == \"pending\":\n        pass\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "same variable in different functions should not cross-contaminate"
        );
    }

    #[test]
    fn noqa_suppresses() {
        let src = "def f():\n    if status == \"active\":  # noqa\n        pass\n    if status == \"inactive\":\n        pass\n    if status == \"pending\":\n        pass\n";
        let findings = parse_and_check(src);
        // One suppressed, only 2 remain — below threshold
        assert!(findings.is_empty());
    }
}
