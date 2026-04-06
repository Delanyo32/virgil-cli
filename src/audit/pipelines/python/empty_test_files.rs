use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_noqa_suppressed, is_test_file};

use super::primitives::{
    compile_class_def_query, compile_function_def_query, extract_snippet, find_capture_index,
    node_text,
};

pub struct EmptyTestFilesPipeline {
    fn_query: Arc<Query>,
    class_query: Arc<Query>,
}

impl EmptyTestFilesPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_query: compile_function_def_query()?,
            class_query: compile_class_def_query()?,
        })
    }
}

impl GraphPipeline for EmptyTestFilesPipeline {
    fn name(&self) -> &str {
        "empty_test_files"
    }

    fn description(&self) -> &str {
        "Detects test files that contain no test functions"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        // Only check test files
        if !is_test_file(file_path) {
            return Vec::new();
        }

        // Exclude conftest.py and __init__.py — they legitimately have no test functions
        let file_name = std::path::Path::new(file_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");
        if file_name == "conftest.py" || file_name == "__init__.py" {
            return Vec::new();
        }

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_query, "fn_name");

        let mut test_fn_count = 0usize;

        while let Some(m) = matches.next() {
            let name_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
            if let Some(cap) = name_cap {
                let fn_name = node_text(cap.node, source);
                if fn_name.starts_with("test_") {
                    test_fn_count += 1;
                }
            }
        }

        if test_fn_count > 0 {
            return Vec::new();
        }

        // Also check for Test* classes (unittest.TestCase subclasses, pytest test classes)
        let mut class_cursor = QueryCursor::new();
        let mut class_matches =
            class_cursor.matches(&self.class_query, tree.root_node(), source);
        let class_name_idx = find_capture_index(&self.class_query, "class_name");

        while let Some(m) = class_matches.next() {
            let name_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_name_idx);
            if let Some(cap) = name_cap {
                let class_name = node_text(cap.node, source);
                if class_name.starts_with("Test") {
                    return Vec::new();
                }
            }
        }

        // Check noqa suppression on the file root
        if is_noqa_suppressed(source, tree.root_node(), self.name()) {
            return Vec::new();
        }

        let snippet = extract_snippet(source, tree.root_node(), 3);
        vec![AuditFinding {
            file_path: file_path.to_string(),
            line: 1,
            column: 1,
            severity: "info".to_string(),
            pipeline: self.name().to_string(),
            pattern: "empty_test_file".to_string(),
            message: format!(
                "test file `{file_path}` contains no `test_*` functions — may be an abandoned stub or discovery file"
            ),
            snippet,
        }]
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
        let pipeline = EmptyTestFilesPipeline::new().unwrap();
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

    // --- detection tests ---

    #[test]
    fn empty_test_files_detects_file_with_only_import() {
        let src = "import pytest\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_test_file");
        assert_eq!(findings[0].severity, "info");
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[0].column, 1);
    }

    #[test]
    fn empty_test_files_detects_empty_file() {
        let src = "";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_test_file");
    }

    #[test]
    fn empty_test_files_detects_file_with_only_helpers() {
        let src = "import pytest\n\ndef helper():\n    return 42\n\ndef setup():\n    pass\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "empty_test_file");
    }

    // --- exclusion tests ---

    #[test]
    fn empty_test_files_skips_conftest() {
        let src = "import pytest\n\n@pytest.fixture\ndef client():\n    return None\n";
        let findings = parse_and_check_path(src, "tests/conftest.py");
        assert!(
            findings.is_empty(),
            "should skip conftest.py, got: {:?}",
            findings
        );
    }

    #[test]
    fn empty_test_files_skips_init() {
        let src = "";
        let findings = parse_and_check_path(src, "tests/__init__.py");
        assert!(
            findings.is_empty(),
            "should skip __init__.py, got: {:?}",
            findings
        );
    }

    #[test]
    fn empty_test_files_skips_non_test_file() {
        let src = "def helper():\n    pass\n";
        let findings = parse_and_check_path(src, "src/utils.py");
        assert!(
            findings.is_empty(),
            "should skip non-test files, got: {:?}",
            findings
        );
    }

    #[test]
    fn empty_test_files_does_not_flag_file_with_test_function() {
        let src = "def test_something():\n    assert True\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag file with test function, got: {:?}",
            findings
        );
    }

    #[test]
    fn empty_test_files_does_not_flag_file_with_multiple_test_functions() {
        let src = "def test_a():\n    assert 1\n\ndef test_b():\n    assert 2\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag file with multiple test functions, got: {:?}",
            findings
        );
    }

    // --- pipeline metadata ---

    #[test]
    fn empty_test_files_pipeline_name() {
        let pipeline = EmptyTestFilesPipeline::new().unwrap();
        assert_eq!(pipeline.name(), "empty_test_files");
    }

    #[test]
    fn empty_test_files_pipeline_description() {
        let pipeline = EmptyTestFilesPipeline::new().unwrap();
        assert!(!pipeline.description().is_empty());
    }

    #[test]
    fn test_class_with_methods_not_empty() {
        let src = "class TestSuite:\n    def test_something(self):\n        assert True\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag file with Test* class, got: {:?}",
            findings
        );
    }

    #[test]
    fn unittest_testcase_not_empty() {
        let src = "import unittest\n\nclass TestMyFeature(unittest.TestCase):\n    def test_x(self):\n        pass\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should not flag file with TestCase subclass, got: {:?}",
            findings
        );
    }

    #[test]
    fn noqa_suppresses_empty_test() {
        let src = "# noqa\nimport os\n";
        let findings = parse_and_check(src);
        assert!(
            findings.is_empty(),
            "should suppress with # noqa, got: {:?}",
            findings
        );
    }
}
