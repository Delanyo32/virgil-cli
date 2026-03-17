use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::typescript_primitives::{
    compile_interface_declaration_query, extract_snippet, find_capture_index, node_text,
};

pub struct MutableTypesPipeline {
    query: Arc<Query>,
    name_idx: usize,
    body_idx: usize,
}

impl MutableTypesPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_interface_declaration_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        let body_idx = find_capture_index(&query, "body");
        Ok(Self {
            query,
            name_idx,
            body_idx,
        })
    }
}

impl Pipeline for MutableTypesPipeline {
    fn name(&self) -> &str {
        "mutable_types"
    }

    fn description(&self) -> &str {
        "Detects interfaces where all properties are mutable (no `readonly`)"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let iface_name = m
                .captures
                .iter()
                .find(|c| c.index as usize == self.name_idx)
                .map(|c| node_text(c.node, source))
                .unwrap_or("<anonymous>");

            let body_node = match m
                .captures
                .iter()
                .find(|c| c.index as usize == self.body_idx)
            {
                Some(c) => c.node,
                None => continue,
            };

            let mut total_props = 0u32;
            let mut mutable_props = 0u32;
            let mut body_cursor = body_node.walk();

            for child in body_node.named_children(&mut body_cursor) {
                if child.kind() == "property_signature" {
                    total_props += 1;
                    let has_readonly = has_readonly_modifier(child, source);
                    if !has_readonly {
                        mutable_props += 1;
                    }
                }
            }

            // Only flag if >3 properties and ALL are mutable
            if total_props > 3 && mutable_props == total_props {
                let decl_node = m.captures.first().map(|c| c.node).unwrap_or(body_node);
                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "mutable_interface".to_string(),
                    message: format!(
                        "Interface `{iface_name}` has {total_props} mutable properties — consider `readonly` to prevent accidental mutation"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        findings
    }
}

fn has_readonly_modifier(node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() && node_text(child, source) == "readonly" {
            return true;
        }
        // Also check for `accessibility_modifier` or similar named nodes
        if child.kind() == "accessibility_modifier" && node_text(child, source) == "readonly" {
            return true;
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
        let pipeline = MutableTypesPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_all_mutable() {
        let src = r#"
interface User {
    id: string;
    name: string;
    email: string;
    age: number;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mutable_interface");
    }

    #[test]
    fn skips_with_readonly() {
        let src = r#"
interface User {
    readonly id: string;
    name: string;
    email: string;
    age: number;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_small_interface() {
        let src = r#"
interface Point {
    x: number;
    y: number;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn boundary_exactly_three_props() {
        let src = r#"
interface Triple {
    a: string;
    b: number;
    c: boolean;
}
"#;
        let findings = parse_and_check(src);
        // 3 is not > 3
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        MutableTypesPipeline::new(Language::Tsx).unwrap();
    }
}
