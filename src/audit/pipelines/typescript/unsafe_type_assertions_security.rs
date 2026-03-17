use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_as_expression_query, compile_function_query, compile_type_predicate_function_query,
    extract_snippet, find_capture_index, node_text,
};

pub struct UnsafeTypeAssertionsSecurityPipeline {
    type_predicate_query: Arc<Query>,
    function_query: Arc<Query>,
    #[allow(dead_code)]
    as_expr_query: Arc<Query>,
}

impl UnsafeTypeAssertionsSecurityPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            type_predicate_query: compile_type_predicate_function_query(language)?,
            function_query: compile_function_query(language)?,
            as_expr_query: compile_as_expression_query(language)?,
        })
    }
}

impl Pipeline for UnsafeTypeAssertionsSecurityPipeline {
    fn name(&self) -> &str {
        "unsafe_type_assertions_security"
    }

    fn description(&self) -> &str {
        "Detects type predicate functions with `as any`, generic functions returning unvalidated JSON.parse"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Type predicate function with `as any` in body
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.type_predicate_query, tree.root_node(), source);
            let body_idx = find_capture_index(&self.type_predicate_query, "body");
            let func_idx = find_capture_index(&self.type_predicate_query, "func");

            while let Some(m) = matches.next() {
                let body_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == body_idx)
                    .map(|c| c.node);
                let func_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == func_idx)
                    .map(|c| c.node);

                if let (Some(body), Some(func)) = (body_node, func_node) {
                    let body_text = node_text(body, source);
                    if body_text.contains("as any") {
                        let start = func.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "type_predicate_as_any".to_string(),
                            message:
                                "Type predicate function uses `as any` — guard may lie about runtime type"
                                    .to_string(),
                            snippet: extract_snippet(source, func, 3),
                        });
                    }
                }
            }
        }

        // Generic function returning JSON.parse without validation
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.function_query, tree.root_node(), source);
            let func_idx = find_capture_index(&self.function_query, "func");
            let params_idx = find_capture_index(&self.function_query, "params");

            while let Some(m) = matches.next() {
                let func_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == func_idx)
                    .map(|c| c.node);
                let _params_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == params_idx)
                    .map(|c| c.node);

                if let Some(func) = func_node {
                    // Check if function has type parameters (generic)
                    let has_type_params = func
                        .child_by_field_name("type_parameters")
                        .is_some();

                    if !has_type_params {
                        continue;
                    }

                    let func_text = node_text(func, source);

                    // Check if body contains JSON.parse and returns it without validation
                    if func_text.contains("JSON.parse") {
                        // Look for return statements with JSON.parse
                        let has_return_json_parse = func_text.contains("return JSON.parse");
                        // Check if there's validation (type guard, schema validation, etc.)
                        let has_validation = func_text.contains("validate")
                            || func_text.contains("schema")
                            || func_text.contains("zod")
                            || func_text.contains("yup")
                            || func_text.contains("joi")
                            || func_text.contains("typeof")
                            || func_text.contains("instanceof");

                        if has_return_json_parse && !has_validation {
                            let start = func.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "generic_unvalidated_parse".to_string(),
                                message: "Generic function returns `JSON.parse()` without runtime validation — caller trusts arbitrary shape".to_string(),
                                snippet: extract_snippet(source, func, 3),
                            });
                        }
                    }
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
        let lang = Language::TypeScript;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&lang.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UnsafeTypeAssertionsSecurityPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_type_predicate_with_as_any() {
        let src = r#"
function isUser(value: unknown): value is User {
    return (value as any).name !== undefined;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "type_predicate_as_any");
    }

    #[test]
    fn ignores_type_predicate_without_as_any() {
        let src = r#"
function isUser(value: unknown): value is User {
    return typeof value === 'object' && value !== null && 'name' in value;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_generic_unvalidated_parse() {
        let src = r#"
function parse<T>(json: string): T {
    return JSON.parse(json);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "generic_unvalidated_parse");
    }

    #[test]
    fn ignores_generic_with_validation() {
        let src = r#"
function parse<T>(json: string, schema: Schema<T>): T {
    const data = JSON.parse(json);
    return schema.validate(data);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        UnsafeTypeAssertionsSecurityPipeline::new(Language::Tsx).unwrap();
    }
}
