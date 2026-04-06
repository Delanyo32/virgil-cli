use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_noqa_suppressed, is_test_file};

use super::primitives::{
    compile_function_def_query, extract_snippet, find_capture_index, node_text,
};

pub struct TestAssertionsPipeline {
    fn_query: Arc<Query>,
}

impl TestAssertionsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_def_query()?,
        })
    }
}

/// Recursively check whether a subtree contains any assertion-like statement.
fn contains_assertion(node: tree_sitter::Node, source: &[u8]) -> bool {
    // Direct assert statement: `assert ...`
    if node.kind() == "assert_statement" {
        return true;
    }

    // `self.assert*()`, `mock.assert_called*()`, `pytest.raises(...)` etc
    if node.kind() == "call"
        && let Some(func) = node.child_by_field_name("function")
        && func.kind() == "attribute"
        && let Some(obj) = func.child_by_field_name("object")
        && let Some(attr) = func.child_by_field_name("attribute")
    {
        let obj_text = node_text(obj, source);
        let attr_text = node_text(attr, source);
        if obj_text == "self" && attr_text.starts_with("assert") {
            return true;
        }
        if obj_text == "pytest"
            && (attr_text == "raises" || attr_text == "warns" || attr_text == "approx")
        {
            return true;
        }
        // Mock assertion methods: mock.assert_called_once(), mock.assert_not_called(), etc.
        if attr_text.starts_with("assert_called")
            || attr_text == "assert_not_called"
            || attr_text == "assert_any_call"
        {
            return true;
        }
    }

    // `with pytest.raises(...):`  or `with raises(...):`
    // In tree-sitter Python, a `with` statement has a `with_clause` containing
    // `with_item` nodes whose value may be the call.  We check the entire text
    // of a with_statement's first child (the with_clause) for `raises` or `warns`.
    if node.kind() == "with_statement" {
        let text = node_text(node, source);
        if text.contains("raises") || text.contains("warns") {
            return true;
        }
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && contains_assertion(child, source)
        {
            return true;
        }
    }

    false
}

/// Check if an `assert_statement` is trivial (asserts a constant literal).
/// Trivial patterns: `assert True`, `assert False`, `assert None`, `assert 1`, `assert 0`.
fn is_trivial_assertion(node: tree_sitter::Node, source: &[u8]) -> bool {
    debug_assert_eq!(node.kind(), "assert_statement");

    // The first named child after the `assert` keyword is the expression being asserted.
    // In tree-sitter Python grammar, the structure is:
    //   (assert_statement (expression) [(expression)])  — second optional is the message
    // We want the first named child (the condition).
    if let Some(expr) = node.named_child(0) {
        let text = node_text(expr, source).trim();
        matches!(text, "True" | "False" | "None" | "1" | "0")
    } else {
        false
    }
}

/// Collect all trivial assertion nodes within a subtree.
fn find_trivial_assertions<'a>(
    node: tree_sitter::Node<'a>,
    source: &[u8],
    results: &mut Vec<tree_sitter::Node<'a>>,
) {
    if node.kind() == "assert_statement" && is_trivial_assertion(node, source) {
        results.push(node);
        return;
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            find_trivial_assertions(child, source, results);
        }
    }
}

impl GraphPipeline for TestAssertionsPipeline {
    fn name(&self) -> &str {
        "test_assertions"
    }

    fn description(&self) -> &str {
        "Detects test functions with missing or trivial assertions"
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
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");
        let fn_body_idx = find_capture_index(&self.fn_query, "fn_body");
        let fn_def_idx = find_capture_index(&self.fn_query, "fn_def");

        while let Some(m) = matches.next() {
            let name_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            let body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);
            let def_cap = m.captures.iter().find(|c| c.index as usize == fn_def_idx);

            let (name_cap, body_cap, def_cap) = match (name_cap, body_cap, def_cap) {
                (Some(n), Some(b), Some(d)) => (n, b, d),
                _ => continue,
            };

            let fn_name = node_text(name_cap.node, source);

            // Only check test functions (def test_*)
            if !fn_name.starts_with("test_") {
                continue;
            }

            let body_node = body_cap.node;
            let def_node = def_cap.node;

            if is_noqa_suppressed(source, def_node, self.name()) {
                continue;
            }

            // Skip @pytest.mark.skip and @pytest.mark.xfail decorated tests
            if let Some(parent) = def_node.parent()
                && parent.kind() == "decorated_definition"
            {
                let mut skip = false;
                for i in 0..parent.named_child_count() {
                    if let Some(child) = parent.named_child(i)
                        && child.kind() == "decorator"
                    {
                        let dec_text = node_text(child, source);
                        if dec_text.contains("pytest.mark.skip")
                            || dec_text.contains("pytest.mark.xfail")
                        {
                            skip = true;
                            break;
                        }
                    }
                }
                if skip {
                    continue;
                }
            }

            // Check for missing assertions
            if !contains_assertion(body_node, source) {
                let start = def_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "missing_assertion".to_string(),
                    message: format!(
                        "test function `{fn_name}` contains no assertions — tests should verify expected behavior"
                    ),
                    snippet: extract_snippet(source, def_node, 3),
                });
                continue;
            }

            // Check for trivial assertions
            let mut trivial_nodes = Vec::new();
            find_trivial_assertions(body_node, source, &mut trivial_nodes);

            for trivial_node in trivial_nodes {
                let start = trivial_node.start_position();
                let asserted_text = trivial_node
                    .named_child(0)
                    .map(|e| node_text(e, source).trim().to_string())
                    .unwrap_or_default();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "trivial_assertion".to_string(),
                    message: format!(
                        "trivial assertion `assert {asserted_text}` in `{fn_name}` — assert a meaningful condition instead"
                    ),
                    snippet: extract_snippet(source, trivial_node, 1),
                });
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
        parse_and_check_path(source, "tests/test_example.py")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TestAssertionsPipeline::new().unwrap();
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

    // --- missing_assertion tests ---

    #[test]
    fn test_assertions_detects_missing_assertion_pass() {
        let src = "def test_foo():\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_assertion");
        assert!(findings[0].message.contains("test_foo"));
    }

    #[test]
    fn test_assertions_detects_missing_assertion_no_assert() {
        let src = "def test_bar():\n    x = 1\n    print(x)\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_assertion");
        assert!(findings[0].message.contains("test_bar"));
    }

    #[test]
    fn test_assertions_skips_function_with_assert() {
        let src = "def test_ok():\n    result = compute()\n    assert result == 1\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag test with assert, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_assertions_skips_function_with_pytest_raises() {
        let src =
            "def test_raises():\n    with pytest.raises(ValueError):\n        do_something()\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag test with pytest.raises, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_assertions_skips_function_with_self_assert() {
        let src = "def test_method(self):\n    self.assertEqual(1, 1)\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag test with self.assertEqual, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_assertions_skips_function_with_pytest_warns() {
        let src =
            "def test_warning():\n    with pytest.warns(UserWarning):\n        trigger_warning()\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag test with pytest.warns, got: {:?}",
            findings
        );
    }

    // --- trivial_assertion tests ---

    #[test]
    fn test_assertions_detects_trivial_assert_true() {
        let src = "def test_trivial():\n    assert True\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_assertion");
        assert!(findings[0].message.contains("assert True"));
    }

    #[test]
    fn test_assertions_detects_trivial_assert_false() {
        let src = "def test_trivial_false():\n    assert False\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_assertion");
    }

    #[test]
    fn test_assertions_detects_trivial_assert_none() {
        let src = "def test_trivial_none():\n    assert None\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_assertion");
    }

    #[test]
    fn test_assertions_detects_trivial_assert_one() {
        let src = "def test_trivial_one():\n    assert 1\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_assertion");
    }

    #[test]
    fn test_assertions_skips_real_assertion() {
        let src = "def test_real():\n    assert x == 1\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag real assertion, got: {:?}",
            findings
        );
    }

    // --- non-test file exclusion ---

    #[test]
    fn test_assertions_skips_non_test_file() {
        let src = "def test_foo():\n    pass\n";
        let findings = parse_and_check_path(src, "src/utils.py");
        assert!(
            findings.is_empty(),
            "should skip non-test files, got: {:?}",
            findings
        );
    }

    // --- non-test functions ---

    #[test]
    fn test_assertions_skips_non_test_function() {
        let src = "def helper():\n    pass\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should skip non-test functions, got: {:?}",
            findings
        );
    }

    // --- pipeline metadata ---

    #[test]
    fn test_assertions_pipeline_name() {
        let pipeline = TestAssertionsPipeline::new().unwrap();
        assert_eq!(pipeline.name(), "test_assertions");
    }

    // --- mixed cases ---

    #[test]
    fn test_assertions_mixed_trivial_and_real() {
        // A function with both a trivial and a real assertion should only flag the trivial one
        let src = "def test_mixed():\n    assert True\n    assert x == 1\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "trivial_assertion");
    }

    #[test]
    fn test_assertions_multiple_test_functions() {
        let src = "\
def test_good():\n    assert result == 42\n\n\
def test_bad():\n    pass\n\n\
def test_trivial():\n    assert True\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"missing_assertion"));
        assert!(patterns.contains(&"trivial_assertion"));
    }

    #[test]
    fn mock_assert_called_counts_as_assertion() {
        let src = "def test_mock():\n    mock_db.assert_called_once_with(42)\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "mock.assert_called_once_with should count as assertion"
        );
    }

    #[test]
    fn mock_assert_not_called_counts_as_assertion() {
        let src = "def test_mock():\n    mock_service.assert_not_called()\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn pytest_mark_skip_suppresses() {
        let src = "@pytest.mark.skip\ndef test_skipped():\n    pass\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "@pytest.mark.skip should suppress missing assertion"
        );
    }

    #[test]
    fn pytest_mark_xfail_suppresses() {
        let src = "@pytest.mark.xfail\ndef test_known_bug():\n    pass\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "@pytest.mark.xfail should suppress missing assertion"
        );
    }

    #[test]
    fn noqa_suppresses_test() {
        let src = "def test_foo():  # noqa\n    pass\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "# noqa should suppress");
    }
}
