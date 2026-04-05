use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{compile_numeric_literal_query, find_capture_index, has_type_qualifier};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", "-1", "10", "100", "1000", "256", "512", "1024", "2048", "4096",
    "8192", "16384", "32768", "65535", "0xFF", "0xff", "0x80", "0xFFFF", "0xffff",
];

/// C-specific allowed numbers (no HTTP codes, ports, or web-specific values).
const C_ALLOWED_NUMBERS: &[&str] = &[
    "3", "4", "5", "6", "7", "8", "16", "32", "64", "128", "255",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &[
    "preproc_def",
    "preproc_function_def",
    "enumerator",
    "bitfield_clause",
    "field_declaration",
    "array_declarator",
    "initializer_list",
    "case_statement",
    // static_assert / _Static_assert are parsed as function calls in tree-sitter C
];

pub struct CMagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl CMagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: compile_numeric_literal_query()?,
        })
    }

    fn is_exempt_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            let kind = parent.kind();

            if EXEMPT_ANCESTOR_KINDS.contains(&kind) {
                return true;
            }

            // Exempt: declaration with const qualifier
            if kind == "declaration" && has_type_qualifier(parent, source, "const") {
                return true;
            }

            current = parent.parent();
        }

        // Skip if in subscript/index expression
        if let Some(parent) = node.parent()
            && parent.kind() == "subscript_expression"
            && let Some(index_child) = parent.child_by_field_name("index")
            && index_child.id() == node.id()
        {
            return true;
        }

        // Skip if operand of bit shift (e.g., x << 24)
        if let Some(parent) = node.parent()
            && parent.kind() == "binary_expression"
        {
            if let Some(op_node) = parent.child_by_field_name("operator") {
                let op = op_node.utf8_text(source).unwrap_or("");
                if op == "<<" || op == ">>" {
                    return true;
                }
            } else {
                // Fallback: check parent text for shift operators
                let text = parent.utf8_text(source).unwrap_or("");
                if text.contains("<<") || text.contains(">>") {
                    return true;
                }
            }
        }

        // Skip if inside _Static_assert or static_assert (parsed as call_expression)
        if let Some(parent) = node.parent() {
            let mut cur = Some(parent);
            while let Some(p) = cur {
                if p.kind() == "call_expression"
                    && let Some(func) = p.child_by_field_name("function")
                {
                    let fname = func.utf8_text(source).unwrap_or("");
                    if fname == "_Static_assert" || fname == "static_assert" {
                        return true;
                    }
                }
                cur = p.parent();
            }
        }

        false
    }
}

impl GraphPipeline for CMagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals outside const/#define/enum contexts that should be named constants"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.numeric_query, tree.root_node(), source);

        let number_idx = find_capture_index(&self.numeric_query, "number");

        const MAX_FINDINGS_PER_FILE: usize = 200;

        while let Some(m) = matches.next() {
            if findings.len() >= MAX_FINDINGS_PER_FILE {
                break;
            }
            let num_cap = m.captures.iter().find(|c| c.index as usize == number_idx);

            if let Some(num_cap) = num_cap {
                let value = num_cap.node.utf8_text(source).unwrap_or("");

                if EXCLUDED_VALUES.contains(&value) || C_ALLOWED_NUMBERS.contains(&value) {
                    continue;
                }

                if Self::is_exempt_context(num_cap.node, source) {
                    continue;
                }

                if is_nolint_suppressed(source, num_cap.node, self.name()) {
                    continue;
                }

                let start = num_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "magic_number".to_string(),
                    message: format!(
                        "magic number `{value}` — consider extracting to a named constant"
                    ),
                    snippet: value.to_string(),
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
        let pipeline = CMagicNumbersPipeline::new().unwrap();
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
    fn detects_magic_number_in_function() {
        let src = "void f() { int x = 42; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_number");
        assert!(findings[0].message.contains("42"));
    }

    #[test]
    fn skips_define() {
        let src = "#define MAX_SIZE 100";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_const() {
        let src = "const int MAX = 100;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_enum() {
        let src = "enum { FOO = 42, BAR = 99 };";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let src = "void f() { int x = 0; int y = 1; int z = 2; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_array_index() {
        let src = "void f() { int x = arr[3]; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_switch_case() {
        let src = r#"
void f(int x) {
    switch(x) {
        case 42: break;
        case 99: break;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_bit_shift_operand() {
        let src = "void f() { int x = val << 24; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_static_assert() {
        let src = "void f() { _Static_assert(sizeof(int) == 42, \"bad\"); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_hex_magic() {
        let src = "void f() { int x = 0xDEADBEEF; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn nolint_suppresses() {
        let src = "void f() { int x = 42; } // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
