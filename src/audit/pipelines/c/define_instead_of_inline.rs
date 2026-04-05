use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{compile_preproc_function_def_query, extract_snippet, find_capture_index};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

/// Check if a macro body contains features that prevent replacement with inline functions.
fn cannot_be_inline(body: &str) -> bool {
    // Token pasting
    if body.contains("##") {
        return true;
    }
    // Stringification: # followed by a letter (parameter name), not ## or #include
    // Check for lone # that isn't part of ##
    let bytes = body.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' {
            // Skip if part of ##
            if i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                continue;
            }
            if i > 0 && bytes[i - 1] == b'#' {
                continue;
            }
            // Lone # followed by an identifier char = stringification
            if i + 1 < bytes.len() && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
            {
                return true;
            }
        }
    }
    // Variadic macros
    if body.contains("__VA_ARGS__") || body.contains("__VA_OPT__") {
        return true;
    }
    // do-while wrapper pattern (statement macro)
    let trimmed = body.trim();
    if trimmed.starts_with("do") && trimmed.contains("while") {
        return true;
    }
    // Control flow from enclosing scope
    for keyword in &["return", "break", "continue", "goto"] {
        // Check as whole word (not substring)
        for (i, _) in body.match_indices(keyword) {
            let before_ok = i == 0
                || !body.as_bytes()[i - 1].is_ascii_alphanumeric()
                    && body.as_bytes()[i - 1] != b'_';
            let end = i + keyword.len();
            let after_ok = end >= body.len()
                || !body.as_bytes()[end].is_ascii_alphanumeric() && body.as_bytes()[end] != b'_';
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

pub struct DefineInsteadOfInlinePipeline {
    macro_query: Arc<Query>,
}

impl DefineInsteadOfInlinePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            macro_query: compile_preproc_function_def_query()?,
        })
    }

    /// Extract the macro body text from the full preproc_function_def node.
    /// The body is everything after the parameters closing paren.
    fn extract_body<'a>(def_node: tree_sitter::Node<'a>, source: &'a [u8]) -> &'a str {
        // Try to get the value field directly
        if let Some(value_node) = def_node.child_by_field_name("value") {
            return value_node.utf8_text(source).unwrap_or("");
        }
        // Fallback: get full text and strip up to the closing paren of params
        let full = def_node.utf8_text(source).unwrap_or("");
        if let Some(paren_pos) = full.find(')') {
            let body = &full[paren_pos + 1..];
            body.trim()
        } else {
            ""
        }
    }
}

impl GraphPipeline for DefineInsteadOfInlinePipeline {
    fn name(&self) -> &str {
        "define_instead_of_inline"
    }

    fn description(&self) -> &str {
        "Detects function-like #define macros that could be inline functions for better type safety"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.macro_query, tree.root_node(), source);

        let macro_name_idx = find_capture_index(&self.macro_query, "macro_name");
        let macro_def_idx = find_capture_index(&self.macro_query, "macro_def");

        while let Some(m) = matches.next() {
            let name_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == macro_name_idx);
            let def_cap = m
                .captures
                .iter()
                .find(|c| c.index as usize == macro_def_idx);

            if let (Some(name_cap), Some(def_cap)) = (name_cap, def_cap) {
                let macro_name = name_cap.node.utf8_text(source).unwrap_or("");
                let body = Self::extract_body(def_cap.node, source);

                // Skip macros that cannot be replaced with inline functions
                if cannot_be_inline(body) {
                    continue;
                }

                if is_nolint_suppressed(source, def_cap.node, self.name()) {
                    continue;
                }

                let start = def_cap.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "function_like_macro".to_string(),
                    message: format!(
                        "function-like macro `{macro_name}` — consider using an inline function for type safety"
                    ),
                    snippet: extract_snippet(source, def_cap.node, 1),
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
        let pipeline = DefineInsteadOfInlinePipeline::new().unwrap();
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
    fn detects_simple_arithmetic_macro() {
        let src = "#define DOUBLE(x) ((x) * 2)";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "function_like_macro");
        assert!(findings[0].message.contains("DOUBLE"));
    }

    #[test]
    fn skips_value_macro() {
        let src = "#define MAX 100";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_stringify_macro() {
        let src = "#define STR(x) #x";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_token_paste_macro() {
        let src = "#define CONCAT(a, b) a##b";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_variadic_macro() {
        let src = "#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_control_flow_macro() {
        let src = "#define CHECK(x) if(!(x)) return -1";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_do_while_wrapper() {
        let src = "#define SWAP(a,b) do { int t = a; a = b; b = t; } while(0)";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_goto_macro() {
        let src = "#define FAIL() goto error";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses() {
        // NOLINT on line above
        let src = "// NOLINT\n#define ADD(a, b) ((a) + (b))";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
