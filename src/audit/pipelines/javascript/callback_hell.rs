use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::extract_snippet;

const NESTING_THRESHOLD: usize = 3;

pub struct CallbackHellPipeline;

impl CallbackHellPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn walk_tree(
        node: tree_sitter::Node,
        callback_depth: usize,
        source: &[u8],
        file_path: &str,
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
            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "warning".to_string(),
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
            Self::walk_tree(child, new_depth, source, file_path, findings);
        }
    }
}

impl Pipeline for CallbackHellPipeline {
    fn name(&self) -> &str {
        "callback_hell"
    }

    fn description(&self) -> &str {
        "Detects deeply nested callbacks (>3 levels) — callback hell anti-pattern"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        Self::walk_tree(tree.root_node(), 0, source, file_path, &mut findings);
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
}
