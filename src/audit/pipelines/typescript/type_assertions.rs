use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_as_expression_query, extract_snippet, is_test_file, is_ts_suppressed, node_text,
};

pub struct TypeAssertionsPipeline {
    query: Arc<Query>,
}

impl TypeAssertionsPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            query: compile_as_expression_query(language)?,
        })
    }
}

impl Pipeline for TypeAssertionsPipeline {
    fn name(&self) -> &str {
        "type_assertions"
    }

    fn description(&self) -> &str {
        "Detects `as` type assertions which override the type checker"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);
        let in_test = is_test_file(file_path);

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.first() {
                let node = cap.node;
                let start = node.start_position();

                // as_expression has positional children: child(0) = expression, last named = target type
                let expr_child = node.named_child(0);
                let type_child = node.named_child(1);

                let target_type_text = type_child.map(|t| node_text(t, source)).unwrap_or("");

                // Skip safe/idiomatic casts (`as const`, `as unknown`)
                // Also check the full node text to catch cases where the type child is unnamed
                let node_full_text = node_text(node, source);
                if target_type_text == "const"
                    || target_type_text == "unknown"
                    || node_full_text.ends_with(" as const")
                    || node_full_text.ends_with(" as unknown")
                {
                    continue;
                }
                // Skip suppressed nodes
                if is_ts_suppressed(source, node) {
                    continue;
                }

                // Check if expression child is also as_expression (double assertion)
                let is_double = expr_child
                    .map(|e| e.kind() == "as_expression")
                    .unwrap_or(false);

                // Skip inner as_expression if we already reported the outer double
                if node.parent().map(|p| p.kind()) == Some("as_expression") {
                    continue;
                }

                let (pattern, severity, message) = if is_double {
                    (
                        "double_assertion",
                        "warning",
                        "Double type assertion (`as X as Y`) circumvents type checking entirely",
                    )
                } else if target_type_text == "any" {
                    (
                        "as_any",
                        "warning",
                        "`as any` silences the type checker — consider narrowing with type guards instead",
                    )
                } else if in_test {
                    (
                        "test_type_assertion",
                        "info",
                        "Type assertion in test file — consider using proper test fixtures or type-safe builders",
                    )
                } else {
                    (
                        "type_assertion",
                        "info",
                        "`as` assertion overrides the type checker — ensure this is intentional and safe",
                    )
                };

                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message: message.to_string(),
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

    fn parse_and_check_with_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TypeAssertionsPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
    }

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_with_path(source, "test.ts")
    }

    #[test]
    fn detects_as_any() {
        let findings = parse_and_check("let x = y as any;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "as_any");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn detects_type_assertion() {
        let findings = parse_and_check("let x = y as string;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "type_assertion");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn detects_double_assertion() {
        let findings = parse_and_check("let x = y as unknown as string;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "double_assertion");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn test_file_uses_test_pattern() {
        let findings = parse_and_check_with_path("let x = y as string;", "src/foo.test.ts");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "test_type_assertion");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn skips_as_const() {
        let findings = parse_and_check("const x = ['a', 'b'] as const;");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_as_unknown() {
        let findings = parse_and_check("const x = someValue as unknown;");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppression_skips_assertion() {
        let findings = parse_and_check("// @ts-ignore\nlet x = y as string;");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppression_skips_as_any() {
        let findings = parse_and_check("// @ts-expect-error\nlet x = y as any;");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_assertion_clean() {
        let findings = parse_and_check("let x: string = 'hello';");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        let pipeline = TypeAssertionsPipeline::new(Language::Tsx).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Tsx.tree_sitter_language())
            .unwrap();
        let tree = parser.parse("let x = y as string;", None).unwrap();
        let findings = pipeline.check(&tree, b"let x = y as string;", "test.tsx");
        assert_eq!(findings.len(), 1);
    }
}
