use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_variable_declaration_query, extract_snippet};

pub struct VarUsagePipeline {
    var_query: Arc<Query>,
}

impl VarUsagePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            var_query: compile_variable_declaration_query()?,
        })
    }
}

impl Pipeline for VarUsagePipeline {
    fn name(&self) -> &str {
        "var_usage"
    }

    fn description(&self) -> &str {
        "Detects `var` declarations that should use `let` or `const`"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.var_query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.first() {
                let node = cap.node;
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "var_usage".to_string(),
                    message: "`var` has function scope and hoisting — prefer `let` or `const`"
                        .to_string(),
                    snippet: extract_snippet(source, node, 1),
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
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = VarUsagePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_var_declaration() {
        let findings = parse_and_check("var x = 1;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "var_usage");
    }

    #[test]
    fn skips_let_declaration() {
        let findings = parse_and_check("let x = 1;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_const_declaration() {
        let findings = parse_and_check("const x = 1;");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_vars() {
        let findings = parse_and_check("var x = 1;\nvar y = 2;");
        assert_eq!(findings.len(), 2);
    }
}
