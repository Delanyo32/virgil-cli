use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_spread_in_object_query, extract_snippet, find_capture_index, node_text,
};

pub struct ShallowSpreadCopyPipeline {
    spread_query: Arc<Query>,
}

impl ShallowSpreadCopyPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            spread_query: compile_spread_in_object_query()?,
        })
    }

    /// Check if the object has non-spread properties after the spread.
    /// `{ ...state, key: val }` is the standard immutable update pattern -- suppress.
    fn has_property_overrides(obj_node: Node, spread_node: Node) -> bool {
        let spread_end = spread_node.end_byte();
        let mut cursor = obj_node.walk();
        for child in obj_node.named_children(&mut cursor) {
            // Properties that come after the spread element in source order
            if child.start_byte() > spread_end {
                let kind = child.kind();
                if kind == "pair"
                    || kind == "shorthand_property_identifier"
                    || kind == "method_definition"
                    || kind == "shorthand_property_identifier_pattern"
                {
                    return true;
                }
            }
        }
        false
    }

    /// Check if the spread is inside a JSX expression (React prop spreading is idiomatic).
    fn is_inside_jsx(node: Node) -> bool {
        let mut current = node.parent();
        while let Some(p) = current {
            match p.kind() {
                "jsx_expression" | "jsx_element" | "jsx_self_closing_element"
                | "jsx_opening_element" => return true,
                _ => current = p.parent(),
            }
        }
        false
    }

    /// Check if the spread is inside a return statement of a function that looks like a reducer.
    fn is_in_reducer_context(node: Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(p) = current {
            match p.kind() {
                "function_declaration" => {
                    if let Some(name) = p.child_by_field_name("name") {
                        let fn_name = node_text(name, source);
                        let lower = fn_name.to_lowercase();
                        if lower.contains("reducer") || lower.contains("reduce") {
                            return true;
                        }
                    }
                    return false;
                }
                "arrow_function" | "function_expression" => {
                    // Check if assigned to a variable with "reducer" in the name
                    if let Some(parent) = p.parent() {
                        if parent.kind() == "variable_declarator" {
                            if let Some(name) = parent.child_by_field_name("name") {
                                if name.kind() == "identifier" {
                                    let var_name = node_text(name, source);
                                    let lower = var_name.to_lowercase();
                                    if lower.contains("reducer") || lower.contains("reduce") {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                    return false;
                }
                _ => current = p.parent(),
            }
        }
        false
    }
}

impl NodePipeline for ShallowSpreadCopyPipeline {
    fn name(&self) -> &str {
        "shallow_spread_copy"
    }

    fn description(&self) -> &str {
        "Detects `{ ...obj }` shallow copies that may not deeply clone nested objects"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.spread_query, tree.root_node(), source);

        let target_idx = find_capture_index(&self.spread_query, "target");
        let spread_idx = find_capture_index(&self.spread_query, "spread");
        let obj_idx = find_capture_index(&self.spread_query, "obj");

        while let Some(m) = matches.next() {
            let target_cap = m.captures.iter().find(|c| c.index as usize == target_idx);
            let spread_cap = m.captures.iter().find(|c| c.index as usize == spread_idx);
            let obj_cap = m.captures.iter().find(|c| c.index as usize == obj_idx);

            if let (Some(target), Some(spread), Some(obj)) = (target_cap, spread_cap, obj_cap) {
                // Only flag when spread target is a plain identifier (variable reference)
                // Skip function calls like { ...getDefaults() } which produce fresh objects
                if target.node.kind() != "identifier" {
                    continue;
                }

                // Suppress: { ...state, key: val } -- standard immutable update pattern
                if Self::has_property_overrides(obj.node, spread.node) {
                    continue;
                }

                // Suppress: spread inside JSX (React prop spreading is idiomatic)
                if Self::is_inside_jsx(obj.node) {
                    continue;
                }

                // Suppress: inside a reducer function
                if Self::is_in_reducer_context(obj.node, source) {
                    continue;
                }

                if is_nolint_suppressed(source, obj.node, self.name()) {
                    continue;
                }

                let start = obj.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "shallow_spread_copy".to_string(),
                    message: "spread copy is shallow — nested objects are still shared references"
                        .to_string(),
                    snippet: extract_snippet(source, obj.node, 1),
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
        let pipeline = ShallowSpreadCopyPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_spread_of_identifier() {
        let findings = parse_and_check("let copy = { ...obj };");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "shallow_spread_copy");
    }

    #[test]
    fn skips_spread_of_call() {
        let findings = parse_and_check("let copy = { ...getDefaults() };");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_spread_no_findings() {
        let findings = parse_and_check("let obj = { a: 1, b: 2 };");
        assert!(findings.is_empty());
    }

    // --- New tests ---

    #[test]
    fn suppresses_immutable_update_pattern() {
        // { ...state, key: val } is the standard React/Redux immutable update
        let findings = parse_and_check("const next = { ...state, loading: true };");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppresses_reducer_context() {
        let src = "function myReducer(state, action) { return { ...state }; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_bare_spread_copy() {
        // Just { ...obj } with no overrides -- potential shallow copy issue
        let findings = parse_and_check("const copy = { ...original };");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn nolint_suppresses_finding() {
        let findings = parse_and_check("// NOLINT(shallow_spread_copy)\nconst copy = { ...obj };");
        assert!(findings.is_empty());
    }
}
