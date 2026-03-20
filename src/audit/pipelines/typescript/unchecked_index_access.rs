use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{compile_subscript_expression_query, extract_snippet};

pub struct UncheckedIndexAccessPipeline {
    query: Arc<Query>,
}

impl UncheckedIndexAccessPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            query: compile_subscript_expression_query(language)?,
        })
    }
}

impl Pipeline for UncheckedIndexAccessPipeline {
    fn name(&self) -> &str {
        "unchecked_index_access"
    }

    fn description(&self) -> &str {
        "Detects unguarded array/object index access that may return undefined"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.first() {
                let node = cap.node;

                // Skip if inside an if_statement condition (already guarded)
                if is_inside_if_condition(node) {
                    continue;
                }

                // Skip if parent is optional chain expression
                if let Some(parent) = node.parent()
                    && parent.kind() == "optional_chain_expression" {
                        continue;
                    }

                // Skip assignment targets (arr[i] = value)
                if let Some(parent) = node.parent()
                    && parent.kind() == "assignment_expression"
                        && let Some(lhs) = parent.child_by_field_name("left")
                            && lhs.id() == node.id() {
                                continue;
                            }

                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unchecked_index".to_string(),
                    message: "Index access may return `undefined` without `noUncheckedIndexedAccess` — consider optional chaining or bounds checking".to_string(),
                    snippet: extract_snippet(source, node, 1),
                });
            }
        }

        findings
    }
}

fn is_inside_if_condition(node: tree_sitter::Node) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "if_statement"
            && let Some(condition) = parent.child_by_field_name("condition")
                && condition.start_byte() <= node.start_byte()
                    && condition.end_byte() >= node.end_byte()
                {
                    return true;
                }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UncheckedIndexAccessPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_array_index() {
        let findings = parse_and_check("let x = arr[0];");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unchecked_index");
    }

    #[test]
    fn detects_object_index() {
        let findings = parse_and_check("let x = obj[\"key\"];");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_assignment_target() {
        let findings = parse_and_check("arr[0] = 'value';");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_if_condition() {
        let findings = parse_and_check("if (arr[0]) { console.log('exists'); }");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_subscript_clean() {
        let findings = parse_and_check("let x = arr.length;");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        UncheckedIndexAccessPipeline::new(Language::Tsx).unwrap();
    }
}
