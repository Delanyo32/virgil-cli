use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

const SERIALIZATION_FUNCTIONS: &[&str] = &["fwrite", "fread", "write", "send", "sendto"];

/// Well-known C macros/types that start uppercase but are NOT struct typedefs.
const EXCLUDED_UPPERCASE_NAMES: &[&str] = &[
    "FILE", "DIR", "NULL", "EOF", "STDIN", "STDOUT", "STDERR", "TRUE", "FALSE", "BOOL",
    "HANDLE", "DWORD", "WORD", "BYTE", "SIZE_MAX", "INT_MAX", "UINT_MAX",
];

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
            let mut child_cursor = node.walk();
            for child in node.children(&mut child_cursor) {
                if child.kind() == "parenthesized_expression" || child.kind() == "type_descriptor" {
                    let inner_text = node_text(child, source).trim().to_string();
                    let inner = inner_text
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .trim();

                    if inner.contains("struct") {
                        return true;
                    }

                    if Self::has_type_identifier(child, source) {
                        return true;
                    }
                }
            }
        }

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
        if node.kind() == "primitive_type" {
            return false;
        }
        // sizeof(TypedefName) parses as identifier — treat uppercase-starting identifiers
        // as likely type names, but exclude well-known non-struct macros/types
        if node.kind() == "identifier" {
            let text = node.utf8_text(source).unwrap_or("");
            if text.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && !EXCLUDED_UPPERCASE_NAMES.contains(&text)
            {
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

    /// Check if the source has __attribute__((packed)) or #pragma pack before the given line.
    fn is_packed_context(source: &[u8], _call_line: u32) -> bool {
        let source_str = std::str::from_utf8(source).unwrap_or("");
        // Check for __attribute__((packed)) anywhere in the file
        if source_str.contains("__attribute__((packed))") || source_str.contains("__packed") {
            return true;
        }
        // Check for #pragma pack
        if source_str.contains("#pragma pack") {
            return true;
        }
        false
    }
}

impl GraphPipeline for RawStructSerializationPipeline {
    fn name(&self) -> &str {
        "raw_struct_serialization"
    }

    fn description(&self) -> &str {
        "Detects fwrite/fread/write/send with sizeof(struct) which is non-portable due to padding"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
        let args_idx = find_capture_index(&self.call_query, "args");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let fn_cap = m.captures.iter().find(|c| c.index as usize == fn_name_idx);
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

                // Skip if the file uses packed structs
                let call_line = call_cap.node.start_position().row as u32;
                if Self::is_packed_context(source, call_line) {
                    continue;
                }

                if is_nolint_suppressed(source, call_cap.node, self.name()) {
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = RawStructSerializationPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.c",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
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

    #[test]
    fn detects_write_syscall() {
        let src = r#"
struct Msg { int type; char data[64]; };
void f(int fd) {
    struct Msg msg;
    write(fd, &msg, sizeof(struct Msg));
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("write"));
    }

    #[test]
    fn detects_send_network() {
        let src = r#"
struct Packet { int seq; char payload[128]; };
void f(int sock) {
    struct Packet pkt;
    send(sock, &pkt, sizeof(struct Packet), 0);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("send"));
    }

    #[test]
    fn skips_packed_struct() {
        let src = r#"
struct __attribute__((packed)) Record { int id; char name[32]; };
void f() {
    struct Record r;
    fwrite(&r, sizeof(struct Record), 1, fp);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_file_not_flagged() {
        // FILE is a well-known type, not a user struct
        let src = "void f() { fread(buf, sizeof(char), n, fp); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_excluded_uppercase() {
        // sizeof(FILE) should not be flagged
        let src = "void f() { fread(buf, sizeof(FILE), 1, fp); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses() {
        let src = r#"
struct Record { int id; };
void f() {
    struct Record r;
    fwrite(&r, sizeof(struct Record), 1, fp); // NOLINT
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
