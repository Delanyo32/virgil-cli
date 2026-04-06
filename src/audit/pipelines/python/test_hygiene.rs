use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_noqa_suppressed, is_test_context_python, is_test_file};

use super::primitives::{
    compile_call_query, extract_snippet, find_capture_index, node_text,
};

pub struct TestHygienePipeline {
    call_query: Arc<Query>,
}

impl TestHygienePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_query()?,
        })
    }
}

impl GraphPipeline for TestHygienePipeline {
    fn name(&self) -> &str {
        "test_hygiene"
    }

    fn description(&self) -> &str {
        "Detects test hygiene issues: excessive mocking and sleep calls in tests"
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

        // --- Pattern 1: excessive_mocking ---
        self.check_excessive_mocking(tree, source, file_path, &mut findings);

        // --- Pattern 2: sleep_in_test ---
        self.check_sleep_in_test(tree, source, file_path, &mut findings);

        findings
    }
}

impl TestHygienePipeline {
    /// Detect test functions with more than 3 patch decorators.
    fn check_excessive_mocking(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let root = tree.root_node();
        self.walk_for_decorated_defs(root, source, file_path, findings);
    }

    /// Recursively walk the tree looking for `decorated_definition` nodes.
    fn walk_for_decorated_defs(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        if node.kind() == "decorated_definition" {
            self.check_decorated_def(node, source, file_path, findings);
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.walk_for_decorated_defs(child, source, file_path, findings);
            }
        }
    }

    /// Check a single `decorated_definition` for excessive mock decorators.
    fn check_decorated_def(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        // Find the inner function definition
        let inner_def = (0..node.child_count())
            .filter_map(|i| node.child(i))
            .find(|c| c.kind() == "function_definition");

        let inner_def = match inner_def {
            Some(d) => d,
            None => return,
        };

        // Check if this is a test function
        let fn_name = match inner_def.child_by_field_name("name") {
            Some(n) => node_text(n, source),
            None => return,
        };

        if !fn_name.starts_with("test_") {
            return;
        }

        // Count patch decorators (only real mock.patch patterns, not @dispatch/@hotpatch)
        let mut patch_count = 0;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i)
                && child.kind() == "decorator"
            {
                if is_noqa_suppressed(source, child, self.name()) {
                    continue;
                }
                let decorator_text = node_text(child, source);
                if Self::is_mock_patch_decorator(decorator_text) {
                    patch_count += 1;
                }
            }
        }

        if patch_count > 3 {
            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "warning".to_string(),
                pipeline: self.name().to_string(),
                pattern: "excessive_mocking".to_string(),
                message: format!(
                    "test function `{fn_name}` has {patch_count} mock patch decorators — consider simplifying dependencies or using a fixture"
                ),
                snippet: extract_snippet(source, node, 3),
            });
        }
    }

    /// Check if a decorator text is a genuine mock.patch decorator (not @dispatch, @hotpatch, etc.)
    fn is_mock_patch_decorator(text: &str) -> bool {
        // Match: @mock.patch, @patch(, @patch.object, @patch.dict, @unittest.mock.patch
        text.contains("mock.patch")
            || text.contains("@patch(")
            || text.contains("@patch.object")
            || text.contains("@patch.dict")
            || text.starts_with("@patch\n")
            || text == "@patch"
    }

    /// Detect `time.sleep(...)` or `asyncio.sleep(...)` calls inside test functions.
    fn check_sleep_in_test(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_expr_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_expr_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            let (fn_expr_cap, call_cap) = match (fn_expr_cap, call_cap) {
                (Some(f), Some(c)) => (f, c),
                _ => continue,
            };

            let fn_text = node_text(fn_expr_cap.node, source);

            if !matches!(
                fn_text,
                "time.sleep" | "asyncio.sleep" | "trio.sleep" | "anyio.sleep"
            ) {
                continue;
            }

            // Check if this call is inside a test context
            if !is_test_context_python(call_cap.node, source, file_path) {
                continue;
            }

            let call_node = call_cap.node;

            if is_noqa_suppressed(source, call_node, self.name()) {
                continue;
            }

            let start = call_node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "sleep_in_test".to_string(),
                message: format!(
                    "`{fn_text}()` in test slows down the suite — use a mock or event-based synchronisation instead"
                ),
                snippet: extract_snippet(source, call_node, 1),
            });
        }
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
        let pipeline = TestHygienePipeline::new().unwrap();
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

    // --- excessive_mocking tests ---

    #[test]
    fn test_hygiene_detects_excessive_mocking_four_patches() {
        let src = "\
@mock.patch('a.b')
@mock.patch('c.d')
@mock.patch('e.f')
@mock.patch('g.h')
def test_over_mocked():
    pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "excessive_mocking");
        assert_eq!(findings[0].severity, "warning");
        assert!(findings[0].message.contains("test_over_mocked"));
        assert!(findings[0].message.contains("4"));
    }

    #[test]
    fn test_hygiene_detects_mixed_patch_decorators() {
        let src = "\
@patch('a.b')
@patch.object(SomeClass, 'method')
@patch.dict('os.environ', {'KEY': 'val'})
@mock.patch('x.y')
def test_mixed():
    pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "excessive_mocking");
    }

    #[test]
    fn test_hygiene_skips_two_patches() {
        let src = "\
@mock.patch('a.b')
@mock.patch('c.d')
def test_reasonable():
    pass
";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag 2 patches, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_hygiene_skips_three_patches() {
        let src = "\
@mock.patch('a.b')
@mock.patch('c.d')
@mock.patch('e.f')
def test_borderline():
    pass
";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag exactly 3 patches, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_hygiene_skips_non_test_function_with_patches() {
        let src = "\
@mock.patch('a.b')
@mock.patch('c.d')
@mock.patch('e.f')
@mock.patch('g.h')
def helper_function():
    pass
";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag non-test functions, got: {:?}",
            findings
        );
    }

    // --- sleep_in_test tests ---

    #[test]
    fn test_hygiene_detects_time_sleep_in_test() {
        let src = "\
def test_slow():
    setup()
    time.sleep(1)
    assert result
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sleep_in_test");
        assert_eq!(findings[0].severity, "info");
        assert!(findings[0].message.contains("time.sleep"));
    }

    #[test]
    fn test_hygiene_detects_asyncio_sleep_in_test() {
        let src = "\
async def test_async_slow():
    await asyncio.sleep(2)
    assert done
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sleep_in_test");
        assert!(findings[0].message.contains("asyncio.sleep"));
    }

    #[test]
    fn test_hygiene_skips_sleep_in_non_test_file() {
        let src = "\
def test_slow():
    time.sleep(1)
    assert result
";
        let findings = parse_and_check_path(src, "src/utils.py");
        assert!(
            findings.is_empty(),
            "should skip non-test files, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_hygiene_skips_sleep_in_non_test_function() {
        let src = "\
def helper():
    time.sleep(1)
";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag sleep in non-test functions, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_hygiene_detects_sleep_in_test_class_method() {
        let src = "\
class TestSlow:
    def test_with_sleep(self):
        time.sleep(5)
        assert True
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sleep_in_test");
    }

    // --- non-test file exclusion ---

    #[test]
    fn test_hygiene_skips_non_test_file_entirely() {
        let src = "\
@mock.patch('a.b')
@mock.patch('c.d')
@mock.patch('e.f')
@mock.patch('g.h')
def test_over_mocked():
    time.sleep(1)
    pass
";
        let findings = parse_and_check_path(src, "src/production.py");
        assert!(
            findings.is_empty(),
            "should skip non-test files entirely, got: {:?}",
            findings
        );
    }

    // --- pipeline metadata ---

    #[test]
    fn test_hygiene_pipeline_name() {
        let pipeline = TestHygienePipeline::new().unwrap();
        assert_eq!(pipeline.name(), "test_hygiene");
    }

    // --- mixed cases ---

    #[test]
    fn test_hygiene_both_patterns_in_same_file() {
        let src = "\
@mock.patch('a.b')
@mock.patch('c.d')
@mock.patch('e.f')
@mock.patch('g.h')
def test_over_mocked():
    pass

def test_sleepy():
    time.sleep(1)
    assert result
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"excessive_mocking"));
        assert!(patterns.contains(&"sleep_in_test"));
    }

    #[test]
    fn test_hygiene_multiple_sleeps_in_one_test() {
        let src = "\
def test_very_slow():
    time.sleep(1)
    do_thing()
    time.sleep(2)
    assert done
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.pattern == "sleep_in_test"));
    }

    #[test]
    fn custom_decorator_with_patch_not_flagged() {
        let src = "\
@hotpatch
@dispatch
@route_patch
@api_patch
def test_custom():
    assert True
";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "custom decorators containing 'patch' should not be flagged, got: {:?}",
            findings
        );
    }

    #[test]
    fn trio_sleep_detected() {
        let src = "def test_async():\n    trio.sleep(1)\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sleep_in_test");
    }

    #[test]
    fn anyio_sleep_detected() {
        let src = "def test_async():\n    anyio.sleep(1)\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sleep_in_test");
    }

    #[test]
    fn noqa_suppresses_sleep() {
        let src = "def test_x():\n    time.sleep(1)  # noqa\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "# noqa should suppress sleep finding");
    }
}
