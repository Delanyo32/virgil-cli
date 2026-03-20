use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{compile_predefined_type_query, extract_snippet, node_text};

pub struct AnyEscapeHatchPipeline {
    query: Arc<Query>,
}

impl AnyEscapeHatchPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            query: compile_predefined_type_query(language)?,
        })
    }
}

impl Pipeline for AnyEscapeHatchPipeline {
    fn name(&self) -> &str {
        "any_escape_hatch"
    }

    fn description(&self) -> &str {
        "Detects usage of `any` type which bypasses TypeScript's type system"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.first() {
                let node = cap.node;
                let text = node_text(node, source);
                if text != "any" {
                    continue;
                }

                let start = node.start_position();
                let (pattern, message) = classify_any_usage(node);

                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message: message.to_string(),
                    snippet: extract_snippet(source, node.parent().unwrap_or(node), 1),
                });
            }
        }

        findings
    }
}

fn classify_any_usage(node: tree_sitter::Node) -> (&'static str, &'static str) {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "type_arguments" => {
                return (
                    "any_in_generics",
                    "`any` in generic type argument defeats type safety — use a specific type or `unknown`",
                );
            }
            "function_declaration" | "arrow_function" | "method_definition" => {
                // Check if we're in the return type position
                if let Some(return_type) = parent.child_by_field_name("return_type")
                    && is_ancestor_of(return_type, node) {
                        return (
                            "any_return",
                            "Function returns `any` — callers lose type safety. Use a specific return type or `unknown`",
                        );
                    }
                break;
            }
            _ => {
                current = parent;
            }
        }
    }

    (
        "any_annotation",
        "`: any` disables type checking — prefer `unknown` if the type is truly dynamic",
    )
}

fn is_ancestor_of(ancestor: tree_sitter::Node, descendant: tree_sitter::Node) -> bool {
    let mut current = Some(descendant);
    while let Some(node) = current {
        if node.id() == ancestor.id() {
            return true;
        }
        current = node.parent();
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
        let pipeline = AnyEscapeHatchPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_any_annotation() {
        let findings = parse_and_check("let x: any = 1;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "any_annotation");
    }

    #[test]
    fn detects_any_in_generics() {
        let findings = parse_and_check("let x: Array<any> = [];");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "any_in_generics");
    }

    #[test]
    fn detects_any_return_type() {
        let findings = parse_and_check("function foo(): any { return 1; }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "any_return");
    }

    #[test]
    fn skips_other_types() {
        let findings = parse_and_check("let x: string = 'hello';");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_unknown() {
        let findings = parse_and_check("let x: unknown = 1;");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_any() {
        let findings = parse_and_check("let x: any = 1;\nlet y: any = 2;");
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn tsx_compiles() {
        let pipeline = AnyEscapeHatchPipeline::new(Language::Tsx).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Tsx.tree_sitter_language())
            .unwrap();
        let tree = parser.parse("let x: any = 1;", None).unwrap();
        let findings = pipeline.check(&tree, b"let x: any = 1;", "test.tsx");
        assert_eq!(findings.len(), 1);
    }
}
