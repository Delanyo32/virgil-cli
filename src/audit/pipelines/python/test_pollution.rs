use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_test_file;

use super::primitives::{
    compile_class_def_query, extract_snippet, find_capture_index, node_text,
};

pub struct TestPollutionPipeline {
    class_query: Arc<Query>,
}

impl TestPollutionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            class_query: compile_class_def_query()?,
        })
    }
}

/// Known mutable-constructor function names.
const MUTABLE_CALL_NAMES: &[&str] = &[
    "list",
    "dict",
    "set",
    "defaultdict",
    "OrderedDict",
];

/// Check whether a node represents a mutable value expression.
/// Mutable types: `[]` (list), `{}` (dictionary), `set(...)`, `list()`,
/// `dict()`, `defaultdict(...)`, `OrderedDict()`, and `call` nodes
/// whose function resolves to one of `MUTABLE_CALL_NAMES`.
fn is_mutable_value(node: tree_sitter::Node, source: &[u8]) -> bool {
    match node.kind() {
        // Literal `[]`
        "list" => true,
        // Literal `{}`  (Python parses `{}` as a dictionary, not a set)
        "dictionary" => true,
        // Literal `{expr, ...}` set literal
        "set" => true,
        // `list()`, `dict()`, `set()`, `defaultdict(...)`, `OrderedDict()`
        "call" => {
            if let Some(func) = node.child_by_field_name("function") {
                let func_text = node_text(func, source);
                // Plain name: `list()`, `defaultdict()`
                if MUTABLE_CALL_NAMES.contains(&func_text) {
                    return true;
                }
                // Dotted name: `collections.OrderedDict()`, `collections.defaultdict()`
                if func.kind() == "attribute"
                    && let Some(attr) = func.child_by_field_name("attribute")
                {
                    let attr_text = node_text(attr, source);
                    if MUTABLE_CALL_NAMES.contains(&attr_text) {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

impl GraphPipeline for TestPollutionPipeline {
    fn name(&self) -> &str {
        "test_pollution"
    }

    fn description(&self) -> &str {
        "Detects mutable module-level and class-level state in test files that can cause cross-test pollution"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        // Only check test files
        if !is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // --- Pattern 1: global_mutable_test_state ---
        // Walk direct children of the module (root node) looking for assignments
        // to mutable containers.
        let root = tree.root_node();
        for i in 0..root.child_count() {
            if let Some(child) = root.child(i) {
                self.check_assignment_node(
                    child,
                    source,
                    file_path,
                    "global_mutable_test_state",
                    "module-level mutable state `{name}` can cause cross-test pollution when mutated",
                    &mut findings,
                );
            }
        }

        // --- Pattern 2: mutable_class_fixture ---
        // Use class_def_query to find class definitions whose name starts with "Test".
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, root, source);

        let class_name_idx = find_capture_index(&self.class_query, "class_name");
        let class_body_idx = find_capture_index(&self.class_query, "class_body");

        while let Some(m) = matches.next() {
            let name_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_name_idx);
            let body_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_body_idx);

            let (name_cap, body_cap) = match (name_cap, body_cap) {
                (Some(n), Some(b)) => (n, b),
                _ => continue,
            };

            let class_name = node_text(name_cap.node, source);

            // Only check test classes (class Test*)
            if !class_name.starts_with("Test") {
                continue;
            }

            // Walk direct children of the class body
            let body_node = body_cap.node;
            for i in 0..body_node.child_count() {
                if let Some(child) = body_node.child(i) {
                    self.check_assignment_node(
                        child,
                        source,
                        file_path,
                        "mutable_class_fixture",
                        "class-level mutable state `{name}` in test class can cause cross-test pollution",
                        &mut findings,
                    );
                }
            }
        }

        findings
    }
}

impl TestPollutionPipeline {
    /// Check if a statement node is an assignment to a mutable container.
    /// If so, push a finding into `findings`.
    ///
    /// `msg_template` should contain `{name}` which will be replaced with the
    /// variable name.
    fn check_assignment_node(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        pattern: &str,
        msg_template: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        // We look for `expression_statement` containing an `assignment`.
        if node.kind() != "expression_statement" {
            return;
        }

        // The expression_statement should contain an assignment child
        let assignment = (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .find(|c| c.kind() == "assignment");

        let assignment = match assignment {
            Some(a) => a,
            None => return,
        };

        // Get left side (name) and right side (value)
        let left = assignment.child_by_field_name("left");
        let right = assignment.child_by_field_name("right");

        let (left, right) = match (left, right) {
            (Some(l), Some(r)) => (l, r),
            _ => return,
        };

        if !is_mutable_value(right, source) {
            return;
        }

        let var_name = node_text(left, source);
        let start = node.start_position();
        let message = msg_template.replace("{name}", var_name);

        findings.push(AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "warning".to_string(),
            pipeline: self.name().to_string(),
            pattern: pattern.to_string(),
            message,
            snippet: extract_snippet(source, node, 2),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "tests/test_example.py")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TestPollutionPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    // --- global_mutable_test_state tests ---

    #[test]
    fn test_pollution_detects_global_list() {
        let src = "SHARED_DATA = []\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_test_state");
        assert!(findings[0].message.contains("SHARED_DATA"));
    }

    #[test]
    fn test_pollution_detects_global_dict() {
        let src = "CACHE = {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_test_state");
        assert!(findings[0].message.contains("CACHE"));
    }

    #[test]
    fn test_pollution_detects_global_set_call() {
        let src = "ITEMS = set()\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_test_state");
        assert!(findings[0].message.contains("ITEMS"));
    }

    #[test]
    fn test_pollution_detects_global_list_call() {
        let src = "DATA = list()\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_test_state");
        assert!(findings[0].message.contains("DATA"));
    }

    #[test]
    fn test_pollution_detects_global_dict_call() {
        let src = "DATA = dict()\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_test_state");
        assert!(findings[0].message.contains("DATA"));
    }

    #[test]
    fn test_pollution_detects_global_defaultdict() {
        let src = "DATA = defaultdict(list)\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_test_state");
        assert!(findings[0].message.contains("DATA"));
    }

    #[test]
    fn test_pollution_detects_global_collections_ordered_dict() {
        let src = "DATA = collections.OrderedDict()\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_test_state");
        assert!(findings[0].message.contains("DATA"));
    }

    // --- negative: immutable module-level ---

    #[test]
    fn test_pollution_skips_string_constant() {
        let src = "CONSTANT = \"immutable\"\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag string constant, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_pollution_skips_integer_constant() {
        let src = "MAX_RETRIES = 3\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag integer constant, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_pollution_skips_tuple_constant() {
        let src = "ALLOWED = (1, 2, 3)\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag tuple constant, got: {:?}",
            findings
        );
    }

    // --- negative: inside function ---

    #[test]
    fn test_pollution_skips_local_variable_in_function() {
        let src = "def test_foo():\n    items = []\n    items.append(1)\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag local variable inside function, got: {:?}",
            findings
        );
    }

    // --- mutable_class_fixture tests ---

    #[test]
    fn test_pollution_detects_class_level_list() {
        let src = "class TestFoo:\n    data = []\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_class_fixture");
        assert!(findings[0].message.contains("data"));
    }

    #[test]
    fn test_pollution_detects_class_level_dict() {
        let src = "class TestBar:\n    cache = {}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_class_fixture");
        assert!(findings[0].message.contains("cache"));
    }

    #[test]
    fn test_pollution_detects_class_level_set_call() {
        let src = "class TestBaz:\n    items = set()\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_class_fixture");
    }

    // --- negative: immutable class-level ---

    #[test]
    fn test_pollution_skips_immutable_class_attribute() {
        let src = "class TestFoo:\n    timeout = 30\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag immutable class attribute, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_pollution_skips_string_class_attribute() {
        let src = "class TestFoo:\n    name = \"test\"\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag string class attribute, got: {:?}",
            findings
        );
    }

    // --- negative: non-Test class ---

    #[test]
    fn test_pollution_skips_non_test_class() {
        let src = "class Helper:\n    data = []\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag non-Test class, got: {:?}",
            findings
        );
    }

    // --- non-test file exclusion ---

    #[test]
    fn test_pollution_skips_non_test_file() {
        let src = "SHARED_DATA = []\n";
        let findings = parse_and_check_path(src, "src/utils.py");
        assert!(
            findings.is_empty(),
            "should skip non-test files, got: {:?}",
            findings
        );
    }

    // --- pipeline metadata ---

    #[test]
    fn test_pollution_pipeline_name() {
        let pipeline = TestPollutionPipeline::new().unwrap();
        assert_eq!(pipeline.name(), "test_pollution");
    }

    // --- mixed cases ---

    #[test]
    fn test_pollution_multiple_global_mutables() {
        let src = "DATA = []\nCACHE = {}\nMAX = 10\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.pattern == "global_mutable_test_state"));
    }

    #[test]
    fn test_pollution_both_patterns() {
        let src = "GLOBAL_LIST = []\n\nclass TestFoo:\n    data = {}\n\n    def test_something(self):\n        local = []\n        assert len(local) == 0\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"global_mutable_test_state"));
        assert!(patterns.contains(&"mutable_class_fixture"));
    }

    #[test]
    fn test_pollution_severity_is_warning() {
        let src = "DATA = []\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }
}
