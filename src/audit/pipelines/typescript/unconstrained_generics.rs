use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_type_parameter_query, extract_snippet, find_capture_index, node_text,
};

pub struct UnconstrainedGenericsPipeline {
    query: Arc<Query>,
    name_idx: usize,
}

impl UnconstrainedGenericsPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_type_parameter_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        Ok(Self { query, name_idx })
    }
}

impl Pipeline for UnconstrainedGenericsPipeline {
    fn name(&self) -> &str {
        "unconstrained_generics"
    }

    fn description(&self) -> &str {
        "Detects generic type parameters without `extends` constraints in function/method signatures"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let param_node = match m.captures.first() {
                Some(c) => c.node,
                None => continue,
            };

            let type_name = m
                .captures
                .iter()
                .find(|c| c.index as usize == self.name_idx)
                .map(|c| node_text(c.node, source))
                .unwrap_or("<T>");

            // Check if has constraint child
            let has_constraint = param_node.child_by_field_name("constraint").is_some();
            if has_constraint {
                continue;
            }

            // Only flag in function/method signatures, not class-level type params
            if !is_in_function_or_method(param_node) {
                continue;
            }

            let start = param_node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "unconstrained_generic".to_string(),
                message: format!(
                    "Generic parameter `{type_name}` has no `extends` constraint — callers can pass any type"
                ),
                snippet: extract_snippet(source, param_node.parent().unwrap_or(param_node), 1),
            });
        }

        findings
    }
}

fn is_in_function_or_method(node: tree_sitter::Node) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_declaration"
            | "arrow_function"
            | "method_definition"
            | "function_expression" => return true,
            "class_declaration" | "class" | "interface_declaration" => return false,
            _ => current = parent,
        }
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
        let pipeline = UnconstrainedGenericsPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_unconstrained_generic() {
        let findings = parse_and_check("function identity<T>(x: T): T { return x; }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unconstrained_generic");
    }

    #[test]
    fn skips_constrained_generic() {
        let findings = parse_and_check("function foo<T extends object>(x: T): T { return x; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_class_level_generics() {
        let findings = parse_and_check("class Box<T> { value: T; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_method_generic() {
        let src = r#"
class Foo {
    bar<T>(x: T): T { return x; }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_arrow_function_generic() {
        let findings = parse_and_check("const identity = <T>(x: T): T => x;");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn tsx_compiles() {
        UnconstrainedGenericsPipeline::new(Language::Tsx).unwrap();
    }
}
