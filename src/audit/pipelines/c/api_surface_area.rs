use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{
    find_capture_index, has_storage_class, is_c_forward_declaration, is_generated_c_file, node_text,
};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;
const LEAKY_FIELD_THRESHOLD: usize = 4;

const C_SYMBOL_KINDS: &[&str] = &[
    "function_definition",
    "struct_specifier",
    "enum_specifier",
    "union_specifier",
    "type_definition",
    "declaration",
];

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    symbol_query: Arc<Query>,
    struct_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        let symbol_query_str = r#"
[
  (function_definition) @sym
  (declaration) @sym
  (struct_specifier) @sym
  (enum_specifier) @sym
  (union_specifier) @sym
  (type_definition) @sym
]
"#;
        let symbol_query = Query::new(&c_lang(), symbol_query_str)
            .with_context(|| "failed to compile symbol query for C API surface")?;

        // Two alternatives:
        // 1. Named struct: struct Foo { ... };
        // 2. Anonymous typedef struct: typedef struct { ... } Foo;
        let struct_query_str = r#"
[
  (struct_specifier
    name: (type_identifier) @struct_name
    body: (field_declaration_list) @field_list) @struct_def
  (type_definition
    type: (struct_specifier
      body: (field_declaration_list) @field_list) @struct_def
    declarator: (type_identifier) @struct_name)
]
"#;
        let struct_query = Query::new(&c_lang(), struct_query_str)
            .with_context(|| "failed to compile struct query for C API surface")?;

        Ok(Self {
            symbol_query: Arc::new(symbol_query),
            struct_query: Arc::new(struct_query),
        })
    }

    /// Count the number of field declarations in a field_declaration_list node.
    fn count_fields(field_list: tree_sitter::Node) -> usize {
        let mut count = 0;
        let mut cursor = field_list.walk();
        for child in field_list.named_children(&mut cursor) {
            if child.kind() == "field_declaration" {
                count += 1;
            }
        }
        count
    }
}

impl Pipeline for ApiSurfaceAreaPipeline {
    fn name(&self) -> &str {
        "api_surface_area"
    }

    fn description(&self) -> &str {
        "Detects excessive public API and leaky abstraction boundaries"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        // Skip generated files entirely
        if is_generated_c_file(file_path, source) {
            return vec![];
        }

        let mut findings = Vec::new();
        let root = tree.root_node();

        // Pattern 1: excessive_public_api — implementation files (.c) only.
        // Header files are pure export surfaces by design; checking them produces false positives.
        if file_path.ends_with(".c") {
            let mut total_symbols = 0usize;
            let mut exported_count = 0usize;

            let mut cursor = QueryCursor::new();
            let sym_idx = find_capture_index(&self.symbol_query, "sym");
            let mut matches = cursor.matches(&self.symbol_query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index as usize == sym_idx {
                        let node = cap.node;
                        // Only count top-level symbols
                        if !node.parent().is_some_and(|p| p.kind() == "translation_unit") {
                            continue;
                        }
                        // Skip extern declarations — they advertise external symbols, not define them
                        if node.kind() == "declaration"
                            && has_storage_class(node, source, "extern")
                        {
                            continue;
                        }
                        // Skip declaration forward declarations (e.g. void func(void);)
                        if node.kind() == "declaration" && is_c_forward_declaration(node) {
                            continue;
                        }
                        // Skip struct/enum/union specifiers without a body (forward declarations)
                        if (node.kind() == "struct_specifier"
                            || node.kind() == "enum_specifier"
                            || node.kind() == "union_specifier")
                            && node.child_by_field_name("body").is_none()
                        {
                            continue;
                        }
                        total_symbols += 1;
                        if !has_storage_class(node, source, "static") {
                            exported_count += 1;
                        }
                    }
                }
            }

            if total_symbols >= EXCESSIVE_API_MIN_SYMBOLS {
                let ratio = exported_count as f64 / total_symbols as f64;
                if ratio > EXCESSIVE_API_EXPORT_RATIO {
                    // Graduated severity: 80–90% = info, >90% = warning
                    let severity = if ratio > 0.90 { "warning" } else { "info" };
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: 1,
                        column: 1,
                        severity: severity.to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "excessive_public_api".to_string(),
                        message: format!(
                            "Module exports {}/{} symbols ({:.0}% exported, threshold: >{}%)",
                            exported_count,
                            total_symbols,
                            ratio * 100.0,
                            (EXCESSIVE_API_EXPORT_RATIO * 100.0) as u32
                        ),
                        snippet: String::new(),
                    });
                }
            }
        }

        // Pattern 2: leaky_abstraction_boundary — header files (.h) only.
        // Non-static struct definitions with >= 4 fields in headers expose internals.
        if file_path.ends_with(".h") {
            let mut cursor = QueryCursor::new();
            let struct_name_idx = find_capture_index(&self.struct_query, "struct_name");
            let field_list_idx = find_capture_index(&self.struct_query, "field_list");
            let struct_def_idx = find_capture_index(&self.struct_query, "struct_def");

            let mut matches = cursor.matches(&self.struct_query, root, source);
            let mut reported_structs = HashSet::new();

            while let Some(m) = matches.next() {
                let mut struct_name = "";
                let mut struct_line = 0u32;
                let mut field_count = 0usize;
                let mut is_static = false;

                for cap in m.captures {
                    if cap.index as usize == struct_name_idx {
                        struct_name = node_text(cap.node, source);
                        struct_line = cap.node.start_position().row as u32 + 1;
                    }
                    if cap.index as usize == field_list_idx {
                        field_count = Self::count_fields(cap.node);
                    }
                    if cap.index as usize == struct_def_idx {
                        is_static = has_storage_class(cap.node, source, "static");
                        if let Some(parent) = cap.node.parent() {
                            if has_storage_class(parent, source, "static") {
                                is_static = true;
                            }
                        }
                    }
                }

                if !struct_name.is_empty()
                    && !is_static
                    && field_count >= LEAKY_FIELD_THRESHOLD
                    && !reported_structs.contains(struct_name)
                {
                    reported_structs.insert(struct_name.to_string());
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: struct_line,
                        column: 1,
                        severity: "warning".to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "leaky_abstraction_boundary".to_string(),
                        message: format!(
                            "Struct `{}` exposes {} fields in a header file — consider using an opaque pointer",
                            struct_name, field_count
                        ),
                        snippet: String::new(),
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

    fn parse_and_check(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&c_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), file_path)
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::new();
        // 10 non-static + 1 static = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        src.push_str("static void private_func(void) {}\n");
        let findings = parse_and_check(&src, "utils.c");
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
void foo(void) {}
void bar(void) {}
static void baz(void) {}
static void qux(void) {}
"#;
        let findings = parse_and_check(src, "utils.c");
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction_in_header() {
        let src = r#"
struct Connection {
    int socket_fd;
    int buffer_pos;
    int retry_count;
    int max_retries;
    int is_connected;
};
"#;
        let findings = parse_and_check(src, "connection.h");
        assert!(findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }

    #[test]
    fn no_leaky_in_c_file() {
        let src = r#"
struct Connection {
    int socket_fd;
    int buffer_pos;
    int retry_count;
    int max_retries;
    int is_connected;
};
"#;
        let findings = parse_and_check(src, "connection.c");
        assert!(!findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }

    #[test]
    fn no_leaky_for_small_struct() {
        let src = r#"
struct Point {
    int x;
    int y;
};
"#;
        let findings = parse_and_check(src, "geometry.h");
        assert!(!findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }

    #[test]
    fn no_leaky_for_opaque_forward_declaration() {
        let src = r#"
struct Connection;
"#;
        let findings = parse_and_check(src, "connection.h");
        assert!(!findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"));
    }

    // ── New tests ──────────────────────────────────────────────────────────

    #[test]
    fn test_header_file_skips_excessive_public_api() {
        let mut src = String::new();
        for i in 0..15 {
            src.push_str(&format!("void func_{}(void);\n", i));
        }
        let findings = parse_and_check(&src, "api.h");
        assert!(
            !findings.iter().any(|f| f.pattern == "excessive_public_api"),
            "Header files should not be checked for excessive_public_api"
        );
    }

    #[test]
    fn test_c_file_correctly_flagged_for_high_ratio() {
        let mut src = String::new();
        // 12 non-static + 1 static = 13 total, 12/13 = 92.3% > 90% → "warning"
        for i in 0..12 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        src.push_str("static void internal(void) {}\n");
        let findings = parse_and_check(&src, "utils.c");
        assert!(
            findings.iter().any(|f| f.pattern == "excessive_public_api"),
            ".c files with high export ratio should trigger excessive_public_api"
        );
        let f = findings
            .iter()
            .find(|f| f.pattern == "excessive_public_api")
            .unwrap();
        assert_eq!(f.severity, "warning", "92% ratio should be 'warning' severity");
    }

    #[test]
    fn test_extern_declarations_excluded_from_ratio() {
        let mut src = String::new();
        for i in 0..4 {
            src.push_str(&format!("extern int g_{};\n", i));
        }
        for i in 0..11 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check(&src, "module.c");
        let finding = findings
            .iter()
            .find(|f| f.pattern == "excessive_public_api")
            .expect("Should trigger excessive_public_api");
        assert!(
            finding.message.contains("11/11"),
            "Extern declarations must not count — expected '11/11' in: {}",
            finding.message
        );
    }

    #[test]
    fn test_forward_declarations_not_in_denominator() {
        let mut src = String::new();
        for i in 0..5 {
            src.push_str(&format!("struct Type{};\n", i));
        }
        for i in 0..10 {
            src.push_str(&format!("void func_{}(void) {{}}\n", i));
        }
        let findings = parse_and_check(&src, "module.c");
        let finding = findings
            .iter()
            .find(|f| f.pattern == "excessive_public_api")
            .expect("Should trigger excessive_public_api");
        assert!(
            finding.message.contains("10/10"),
            "Forward declarations must not count — expected '10/10' in: {}",
            finding.message
        );
    }

    #[test]
    fn test_generated_header_suppressed() {
        let src = "/* Auto-generated by autoconf. Do not edit. */\n\
                   void f1(void) {}\nvoid f2(void) {}\nvoid f3(void) {}\n\
                   void f4(void) {}\nvoid f5(void) {}\nvoid f6(void) {}\n\
                   void f7(void) {}\nvoid f8(void) {}\nvoid f9(void) {}\n\
                   void f10(void) {}\n";
        let findings = parse_and_check(src, "config.h");
        assert!(findings.is_empty(), "Generated files should produce 0 findings");
    }

    #[test]
    fn test_small_struct_in_header_not_flagged() {
        let src = "struct Vector3 { float x; float y; float z; };\n";
        let findings = parse_and_check(src, "math.h");
        assert!(
            !findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "3-field struct is below the 4-field threshold"
        );
    }

    #[test]
    fn test_exactly_4_field_struct_flagged() {
        let src = r#"
struct Conn {
    int fd;
    int port;
    int timeout;
    int flags;
};
"#;
        let findings = parse_and_check(src, "conn.h");
        assert!(
            findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "4-field struct should trigger leaky_abstraction_boundary"
        );
    }

    #[test]
    fn test_typedef_struct_flagged() {
        let src = r#"
typedef struct {
    int socket_fd;
    char *host;
    int port;
    int flags;
} Connection;
"#;
        let findings = parse_and_check(src, "conn.h");
        assert!(
            findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "Anonymous typedef struct with 4+ fields should trigger leaky_abstraction_boundary"
        );
    }

    #[test]
    fn test_no_leaky_in_c_implementation_file() {
        let src = "struct Large { int a; int b; int c; int d; int e; };\n";
        let findings = parse_and_check(src, "implementation.c");
        assert!(
            !findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "leaky_abstraction_boundary is .h-only"
        );
    }

    #[test]
    fn test_multiple_structs_in_header_all_flagged() {
        let src = r#"
struct Conn { int fd; int port; int timeout; int retry; int flags; };
struct Req  { int method; char *url; int version; int timeout; int flags; };
struct Resp { int status; int len; char *body; int flags; int version; };
"#;
        let findings = parse_and_check(src, "api.h");
        let count = findings
            .iter()
            .filter(|f| f.pattern == "leaky_abstraction_boundary")
            .count();
        assert_eq!(count, 3, "Three distinct structs with 5 fields each → 3 findings");
    }

    #[test]
    fn test_static_struct_in_header_not_flagged() {
        let src = r#"
static struct InternalState {
    int counter;
    int flags;
    int mode;
    int level;
    int depth;
} g_state;
"#;
        let findings = parse_and_check(src, "internal.h");
        assert!(
            !findings.iter().any(|f| f.pattern == "leaky_abstraction_boundary"),
            "Static struct is file-local and should not trigger leaky_abstraction_boundary"
        );
    }
}
