use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::c_primitives::{compile_call_expression_query, extract_snippet, find_capture_index, node_text};

const SERIALIZATION_FUNCTIONS: &[&str] = &["fwrite", "fread"];
pub struct RawStructSerializationPipeline {
    call_query: Arc<Query>,
}

impl RawStructSerializationPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }

    fn has_struct_sizeof(args_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk argument_list children looking for sizeof_expression
        let mut cursor = args_node.walk();
        for child in args_node.children(&mut cursor) {
            if Self::contains_struct_sizeof(child, source) {
                return true;
            }
        }
        false
    }

    fn contains_struct_sizeof(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "sizeof_expression" {
            // Check if the sizeof operand is a struct type (not primitive)
            let mut child_cursor = node.walk();
            for child in node.children(&mut child_cursor) {
                if child.kind() == "parenthesized_expression" || child.kind() == "type_descriptor"
                {
                    let inner_text = node_text(child, source).trim().to_string();
                    // Remove parens
                    let inner = inner_text
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .trim();

                    // If it contains "struct", it's a struct sizeof
                    if inner.contains("struct") {
                        return true;
                    }

                    // If it's a type_identifier (not a known primitive), flag it
                    if Self::has_type_identifier(child, source) {
                        return true;
                    }
                }
            }
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::contains_struct_sizeof(child, source) {
                return true;
            }
        }
        false
    }

    fn has_type_identifier(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "type_identifier" {
            return true;
        }
        if node.kind() == "struct_specifier" {
            return true;
        }
        // Skip primitive types
        if node.kind() == "primitive_type" {
            return false;
        }
        // sizeof(TypedefName) parses as identifier — treat uppercase-starting identifiers
        // as likely type names (common C convention for typedefs)
        if node.kind() == "identifier" {
            let text = node.utf8_text(source).unwrap_or("");
            if text.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                return true;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::has_type_identifier(child, source) {
                return true;
            }
        }
        false
    }
}

impl Pipeline for RawStructSerializationPipeline {
    fn name(&self) -> &str {
        "raw_struct_serialization"
    }

    fn description(&self) -> &str {
        "Detects fwrite/fread with sizeof(struct) which is non-portable due to padding"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx);
            let args_cap = m.captures.iter().find(|c| c.index as usize == args_idx);
            let call_cap = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(fn_cap), Some(args_cap), Some(call_cap)) = (fn_cap, args_cap, call_cap) {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");

                if !SERIALIZATION_FUNCTIONS.contains(&fn_name) {
                    continue;
                }

                if !Self::has_struct_sizeof(args_cap.node, source) {
                    continue;
                }

                let start = call_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "raw_struct_serialization".to_string(),
                    message: format!(
                        "`{fn_name}()` with sizeof(struct) — struct padding makes this non-portable"
                    ),
                    snippet: extract_snippet(source, call_cap.node, 1),
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RawStructSerializationPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_fwrite_with_struct_sizeof() {
        let src = r#"
struct Record { int id; char name[32]; };
void f() {
    struct Record r;
    fwrite(&r, sizeof(struct Record), 1, fp);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "raw_struct_serialization");
        assert!(findings[0].message.contains("fwrite"));
    }

    #[test]
    fn skips_fwrite_with_primitive_sizeof() {
        let src = "void f() { fwrite(buf, sizeof(char), n, fp); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_fread_with_typedef_sizeof() {
        let src = r#"
typedef struct { int x; } Point;
void f() {
    Point p;
    fread(&p, sizeof(Point), 1, fp);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("fread"));
    }
}
