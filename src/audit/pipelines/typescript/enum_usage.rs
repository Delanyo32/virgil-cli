use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_enum_declaration_query, extract_snippet, find_capture_index, is_dts_file, is_test_file,
    is_ts_suppressed, node_text,
};

pub struct EnumUsagePipeline {
    query: Arc<Query>,
    name_idx: usize,
    body_idx: usize,
}

impl EnumUsagePipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_enum_declaration_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        let body_idx = find_capture_index(&query, "body");
        Ok(Self {
            query,
            name_idx,
            body_idx,
        })
    }
}

impl Pipeline for EnumUsagePipeline {
    fn name(&self) -> &str {
        "enum_usage"
    }

    fn description(&self) -> &str {
        "Detects TypeScript enum declarations — union types or `as const` are often better alternatives"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_dts_file(file_path) {
            return Vec::new();
        }
        let in_test = is_test_file(file_path);

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let enum_name = m
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

            let decl_node = m.captures.first().map(|c| c.node).unwrap_or(body_node);

            if is_const_enum(decl_node, source) {
                continue;
            }
            if is_bitflag_enum(body_node, source) {
                continue;
            }
            if is_ts_suppressed(source, decl_node) {
                continue;
            }

            let mut has_string_value = false;
            let mut body_cursor = body_node.walk();

            for child in body_node.named_children(&mut body_cursor) {
                // Members with values are `enum_assignment` (name + value fields)
                if child.kind() == "enum_assignment"
                    && let Some(value_node) = child.child_by_field_name("value")
                    && matches!(value_node.kind(), "string" | "template_string")
                {
                    has_string_value = true;
                }
            }

            let start = decl_node.start_position();

            if has_string_value {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "string_enum".to_string(),
                    message: format!(
                        "String enum `{enum_name}` — consider `as const` object or string union type for better tree-shaking"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            } else {
                let severity = if in_test { "info" } else { "warning" };
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "numeric_enum".to_string(),
                    message: format!(
                        "Numeric enum `{enum_name}` — enums compile to reverse-mapped objects; prefer union types or `as const`"
                    ),
                    snippet: extract_snippet(source, decl_node, 3),
                });
            }
        }

        findings
    }
}

fn is_const_enum(decl_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = decl_node.walk();
    for child in decl_node.children(&mut cursor) {
        if !child.is_named() && node_text(child, source) == "const" {
            return true;
        }
    }
    false
}

fn is_bitflag_enum(body_node: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = body_node.walk();
    for child in body_node.named_children(&mut cursor) {
        if child.kind() == "enum_assignment" {
            if let Some(value_node) = child.child_by_field_name("value") {
                let text = node_text(value_node, source);
                if text.contains("<<") || text.contains('|') {
                    return true;
                }
            }
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
        let pipeline = EnumUsagePipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = EnumUsagePipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
    }

    #[test]
    fn detects_numeric_enum() {
        let findings = parse_and_check("enum Color { Red, Green, Blue }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "numeric_enum");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn detects_string_enum() {
        let src = r#"enum Direction { Up = "UP", Down = "DOWN" }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "string_enum");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn no_enum_clean() {
        let findings = parse_and_check("type Color = 'red' | 'green' | 'blue';");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_const_enum() {
        let findings = parse_and_check("const enum Direction { Up, Down, Left, Right }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_bitflag_enum_left_shift() {
        let findings = parse_and_check("enum Flags { Read = 1 << 0, Write = 1 << 1 }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_bitflag_enum_pipe_operator() {
        let findings = parse_and_check("enum Combo { ReadWrite = 1 | 2 }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_dts_file() {
        let findings = parse_and_check_path("enum Color { Red, Green }", "src/types.d.ts");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_file_downgrades_severity() {
        let findings = parse_and_check_path("enum Color { Red, Green }", "src/color.test.ts");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn suppression_skips_enum() {
        let findings = parse_and_check("// @ts-ignore\nenum Color { Red, Green }");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        EnumUsagePipeline::new(Language::Tsx).unwrap();
    }
}
