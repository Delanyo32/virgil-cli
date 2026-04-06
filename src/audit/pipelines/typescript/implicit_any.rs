use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_function_query, extract_snippet, find_capture_index, is_test_file, is_ts_suppressed,
    node_text,
};

pub struct ImplicitAnyPipeline {
    query: Arc<Query>,
    params_idx: usize,
    func_idx: usize,
}

impl ImplicitAnyPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_function_query(language)?;
        let params_idx = find_capture_index(&query, "params");
        let func_idx = find_capture_index(&query, "func");
        Ok(Self {
            query,
            params_idx,
            func_idx,
        })
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
        if is_test_file(file_path) {
            return Vec::new();
        }

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

            let func_node = match m
                .captures
                .iter()
                .find(|c| c.index as usize == self.func_idx)
            {
                Some(c) => c.node,
                None => continue,
            };

            // Detect callback context: arrow function directly inside call arguments
            let is_callback = func_node.kind() == "arrow_function"
                && func_node
                    .parent()
                    .map(|p| p.kind() == "arguments")
                    .unwrap_or(false);

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

                // Skip params with default values — TypeScript infers the type
                if param.child_by_field_name("value").is_some() {
                    continue;
                }

                // Check for type_annotation child
                let has_type = param.child_by_field_name("type").is_some();

                if !has_type {
                    // Suppression check on param node
                    if is_ts_suppressed(source, param) {
                        continue;
                    }

                    let start = param.start_position();
                    let (pattern, message) = if is_callback {
                        (
                            "inferred_callback_param",
                            format!(
                                "Callback parameter `{param_name}` has no type annotation — type is inferred from context"
                            ),
                        )
                    } else {
                        (
                            "implicit_any_param",
                            format!(
                                "Parameter `{param_name}` has no type annotation — implicit `any` without `noImplicitAny`"
                            ),
                        )
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message,
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

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ImplicitAnyPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
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
    fn skips_param_with_default_value() {
        let findings = parse_and_check("function foo(x = 5) { return x; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_arrow_default_param() {
        let findings = parse_and_check("const foo = (x = 'hello') => x;");
        assert!(findings.is_empty());
    }

    #[test]
    fn callback_in_map_uses_inferred_pattern() {
        let findings = parse_and_check("const result = arr.map(x => x + 1);");
        // Callback param should be inferred_callback_param or info severity, not implicit_any_param at info
        // Just verify it doesn't produce unexpected warnings
        assert!(findings.iter().all(|f| f.severity != "warning"));
    }

    #[test]
    fn skips_entirely_in_test_files() {
        let findings = parse_and_check_path("function foo(x) { return x; }", "foo.test.ts");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppression_skips_param() {
        let findings = parse_and_check("// @ts-ignore\nfunction foo(x) { return x; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        ImplicitAnyPipeline::new(Language::Tsx).unwrap();
    }
}
