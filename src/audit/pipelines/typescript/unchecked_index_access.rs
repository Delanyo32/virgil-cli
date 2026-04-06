use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_subscript_expression_query, extract_snippet, find_capture_index, is_test_file,
    is_ts_suppressed, node_text,
};

pub struct UncheckedIndexAccessPipeline {
    query: Arc<Query>,
    idx_capture: usize,
}

impl UncheckedIndexAccessPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_subscript_expression_query(language)?;
        let idx_capture = find_capture_index(&query, "idx");
        Ok(Self { query, idx_capture })
    }
}

fn is_constant_integer_index(index_node: tree_sitter::Node, source: &[u8]) -> bool {
    index_node.kind() == "number" && node_text(index_node, source).parse::<u64>().is_ok()
}

fn is_nullish_coalescing_guard(node: tree_sitter::Node) -> bool {
    if let Some(parent) = node.parent() {
        // tree-sitter-typescript uses "binary_expression" for ?? with operator child
        if parent.kind() == "binary_expression" {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if !child.is_named() && child.kind() == "??" {
                    return true;
                }
            }
        }
        // Some grammars use a dedicated node kind
        if parent.kind() == "nullish_coalescing_expression" {
            return true;
        }
    }
    false
}

impl Pipeline for UncheckedIndexAccessPipeline {
    fn name(&self) -> &str {
        "unchecked_index_access"
    }

    fn description(&self) -> &str {
        "Detects unguarded array/object index access that may return undefined"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            // sub capture is first (index 0), idx capture is second
            let sub_node = match m.captures.first() {
                Some(c) => c.node,
                None => continue,
            };

            let idx_node = match m
                .captures
                .iter()
                .find(|c| c.index as usize == self.idx_capture)
            {
                Some(c) => c.node,
                None => continue,
            };

            // Skip if inside an if_statement condition (already guarded)
            if is_inside_if_condition(sub_node) {
                continue;
            }

            // Skip if parent is optional chain expression
            if let Some(parent) = sub_node.parent()
                && parent.kind() == "optional_chain_expression"
            {
                continue;
            }

            // Skip assignment targets (arr[i] = value)
            if let Some(parent) = sub_node.parent()
                && parent.kind() == "assignment_expression"
                && let Some(lhs) = parent.child_by_field_name("left")
                && lhs.id() == sub_node.id()
            {
                continue;
            }

            // Skip if guarded by nullish coalescing (??)
            if is_nullish_coalescing_guard(sub_node) {
                continue;
            }

            // Suppression check
            if is_ts_suppressed(source, sub_node) {
                continue;
            }

            // Constant integer index gets "info", dynamic index keeps existing severity
            let severity = if is_constant_integer_index(idx_node, source) {
                "info"
            } else {
                "info"
            };

            let start = sub_node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: severity.to_string(),
                pipeline: self.name().to_string(),
                pattern: "unchecked_index".to_string(),
                message: "Index access may return `undefined` without `noUncheckedIndexedAccess` — consider optional chaining or bounds checking".to_string(),
                snippet: extract_snippet(source, sub_node, 1),
            });
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

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UncheckedIndexAccessPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
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
    fn constant_index_produces_info_not_warning() {
        let findings = parse_and_check("let x = arr[0];");
        assert!(findings.iter().all(|f| f.severity == "info"));
    }

    #[test]
    fn nullish_coalescing_guard_skips() {
        let findings = parse_and_check("let x = arr[0] ?? 'default';");
        assert!(findings.is_empty());
    }

    #[test]
    fn dynamic_index_still_flagged() {
        let findings = parse_and_check("let x = arr[i];");
        assert!(!findings.is_empty());
    }

    #[test]
    fn skips_test_file() {
        let findings = parse_and_check_path("let x = arr[i];", "foo.test.ts");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppression_skips() {
        let findings = parse_and_check("// @ts-ignore\nlet x = arr[i];");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        UncheckedIndexAccessPipeline::new(Language::Tsx).unwrap();
    }
}
