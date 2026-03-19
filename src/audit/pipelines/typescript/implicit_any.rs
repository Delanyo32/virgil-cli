use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{compile_function_query, extract_snippet, find_capture_index, node_text};

pub struct ImplicitAnyPipeline {
    query: Arc<Query>,
    params_idx: usize,
}

impl ImplicitAnyPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_function_query(language)?;
        let params_idx = find_capture_index(&query, "params");
        Ok(Self { query, params_idx })
    }
}

impl Pipeline for ImplicitAnyPipeline {
    fn name(&self) -> &str {
        "implicit_any"
    }

    fn description(&self) -> &str {
        "Detects function parameters without type annotations (implicit any)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let params_node = match m
                .captures
                .iter()
                .find(|c| c.index as usize == self.params_idx)
            {
                Some(c) => c.node,
                None => continue,
            };

            let mut params_cursor = params_node.walk();
            for param in params_node.named_children(&mut params_cursor) {
                match param.kind() {
                    "required_parameter" | "optional_parameter" => {}
                    _ => continue,
                }

                // Get the parameter name
                let param_name = param
                    .child_by_field_name("pattern")
                    .map(|n| node_text(n, source))
                    .unwrap_or("");

                // Skip `this` parameter
                if param_name == "this" {
                    continue;
                }

                // Skip destructuring patterns (object/array patterns)
                if let Some(pattern_node) = param.child_by_field_name("pattern") {
                    match pattern_node.kind() {
                        "object_pattern" | "array_pattern" => continue,
                        _ => {}
                    }
                }

                // Check for type_annotation child
                let has_type = param.child_by_field_name("type").is_some();

                if !has_type {
                    let start = param.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "implicit_any_param".to_string(),
                        message: format!(
                            "Parameter `{param_name}` has no type annotation — implicit `any` without `noImplicitAny`"
                        ),
                        snippet: extract_snippet(source, param, 1),
                    });
                }
            }
        }

        findings
    }
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
        let pipeline = ImplicitAnyPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_untyped_parameter() {
        let findings = parse_and_check("function foo(x) { return x; }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "implicit_any_param");
    }

    #[test]
    fn skips_typed_parameter() {
        let findings = parse_and_check("function foo(x: number) { return x; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_untyped() {
        let findings = parse_and_check("function foo(a, b, c) {}");
        assert_eq!(findings.len(), 3);
    }

    #[test]
    fn mixed_typed_untyped() {
        let findings = parse_and_check("function foo(a: string, b, c: number) {}");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("`b`"));
    }

    #[test]
    fn skips_arrow_with_types() {
        let findings = parse_and_check("const foo = (x: number) => x;");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_arrow_without_types() {
        let findings = parse_and_check("const foo = (x) => x;");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn tsx_compiles() {
        ImplicitAnyPipeline::new(Language::Tsx).unwrap();
    }
}
