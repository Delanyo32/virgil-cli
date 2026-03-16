use anyhow::Result;
use tree_sitter::Tree;

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::python_primitives::extract_snippet;

const NESTING_THRESHOLD: usize = 4;
const NESTING_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "while_statement",
    "with_statement",
    "try_statement",
];

pub struct DeepNestingPipeline;

impl DeepNestingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn walk_tree(
        node: tree_sitter::Node,
        depth: usize,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let is_nesting = NESTING_KINDS.contains(&node.kind());
        let new_depth = if is_nesting { depth + 1 } else { depth };

        if new_depth > NESTING_THRESHOLD && is_nesting {
            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "warning".to_string(),
                pipeline: "deep_nesting".to_string(),
                pattern: "excessive_nesting_depth".to_string(),
                message: format!(
                    "nesting depth {new_depth} exceeds threshold ({NESTING_THRESHOLD}) — consider early returns or extracting helpers"
                ),
                snippet: extract_snippet(source, node, 2),
            });
            // Stop recursing this branch to avoid duplicate reports
            return;
        }

        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            Self::walk_tree(child, new_depth, source, file_path, findings);
        }
    }
}

impl Pipeline for DeepNestingPipeline {
    fn name(&self) -> &str {
        "deep_nesting"
    }

    fn description(&self) -> &str {
        "Detects deeply nested control flow (>4 levels) — arrow anti-pattern"
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
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = DeepNestingPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_deep_nesting() {
        let src = "\
def foo():
    if True:
        if True:
            if True:
                if True:
                    if True:
                        pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "excessive_nesting_depth");
    }

    #[test]
    fn clean_shallow_nesting() {
        let src = "\
def foo():
    if True:
        if True:
            if True:
                if True:
                    pass
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_mixed_control_flow() {
        let src = "\
def foo():
    for x in items:
        if x:
            while True:
                with ctx:
                    if y:
                        pass
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn clean_flat_function() {
        let src = "def foo():\n    x = 1\n    y = 2\n    return x + y\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
