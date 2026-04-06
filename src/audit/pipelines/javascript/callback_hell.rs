use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::extract_snippet;

const NESTING_THRESHOLD: usize = 3;

/// Test-framework function names whose callbacks should be suppressed.
const TEST_CONTEXT_NAMES: &[&str] = &[
    "describe",
    "it",
    "test",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
];

pub struct CallbackHellPipeline;

impl CallbackHellPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Returns true if any ancestor of `node` is a call_expression whose callee
    /// identifier matches a test-framework name (describe, it, test, etc.).
    fn is_inside_test_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "call_expression" {
                if let Some(func) = parent.child_by_field_name("function") {
                    if func.kind() == "identifier" {
                        if let Ok(name) = func.utf8_text(source) {
                            if TEST_CONTEXT_NAMES.contains(&name) {
                                return true;
                            }
                        }
                    }
                }
            }
            current = parent;
        }
        false
    }

    /// Map callback nesting depth to graduated severity.
    fn severity_for_depth(depth: usize) -> &'static str {
        match depth {
            4..=5 => "info",
            6..=7 => "warning",
            _ => "error", // 8+
        }
    }

    fn walk_tree(
        node: tree_sitter::Node,
        callback_depth: usize,
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let is_callback = (node.kind() == "arrow_function" || node.kind() == "function_expression")
            && node
                .parent()
                .map(|p| p.kind() == "arguments")
                .unwrap_or(false);

        let new_depth = if is_callback {
            callback_depth + 1
        } else {
            callback_depth
        };

        if new_depth > NESTING_THRESHOLD && is_callback {
            // Suppress findings inside test framework contexts
            if Self::is_inside_test_context(node, source) {
                return;
            }

            // Suppress findings with NOLINT comment
            if is_nolint_suppressed(source, node, pipeline_name) {
                return;
            }

            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: Self::severity_for_depth(new_depth).to_string(),
                pipeline: "callback_hell".to_string(),
                pattern: "nested_callback".to_string(),
                message: format!(
                    "callback nesting depth {new_depth} exceeds threshold ({NESTING_THRESHOLD}) — consider async/await or named functions"
                ),
                snippet: extract_snippet(source, node, 2),
            });
            return;
        }

        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            Self::walk_tree(child, new_depth, source, file_path, pipeline_name, findings);
        }
    }
}

impl NodePipeline for CallbackHellPipeline {
    fn name(&self) -> &str {
        "callback_hell"
    }

    fn description(&self) -> &str {
        "Detects deeply nested callbacks (>3 levels) — callback hell anti-pattern"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        Self::walk_tree(tree.root_node(), 0, source, file_path, self.name(), &mut findings);
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CallbackHellPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_deep_callback_nesting() {
        let src = r#"
doA(function() {
    doB(function() {
        doC(function() {
            doD(function() {
                console.log("deep");
            });
        });
    });
});
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "nested_callback");
    }

    #[test]
    fn skips_shallow_callbacks() {
        let src = r#"
doA(function() {
    doB(function() {
        doC(function() {
            console.log("ok");
        });
    });
});
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_arrow_function_nesting() {
        let src = r#"
doA(() => {
    doB(() => {
        doC(() => {
            doD(() => {
                console.log("deep");
            });
        });
    });
});
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "nested_callback");
    }

    #[test]
    fn clean_flat_code() {
        let src = "function foo() { bar(); baz(); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn severity_depth_4_info() {
        // depth 4 = info (threshold is 3, so 4 levels deep triggers)
        let src = r#"
doA(() => {
    doB(() => {
        doC(() => {
            doD(() => {
                console.log("depth 4");
            });
        });
    });
});
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn test_context_suppressed() {
        let src = r#"
describe(() => {
    it(() => {
        doA(() => {
            doB(() => {
                doC(() => {
                    doD(() => {
                        console.log("deep in test");
                    });
                });
            });
        });
    });
});
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0, "findings inside test context should be suppressed");
    }

    #[test]
    fn nolint_suppresses() {
        let src = r#"
doA(() => {
    doB(() => {
        doC(() => {
            // NOLINT(callback_hell)
            doD(() => {
                console.log("suppressed");
            });
        });
    });
});
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 0, "NOLINT comment should suppress finding");
    }
}
