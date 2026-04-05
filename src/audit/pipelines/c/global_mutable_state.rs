use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_declaration_query, extract_snippet, find_capture_index, find_identifier_in_declarator,
    has_storage_class, has_type_qualifier,
};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

pub struct GlobalMutableStatePipeline {
    decl_query: Arc<Query>,
}

impl GlobalMutableStatePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            decl_query: compile_declaration_query()?,
        })
    }

    fn is_function_declarator(node: tree_sitter::Node) -> bool {
        if node.kind() == "function_declarator" {
            return true;
        }
        if let Some(inner) = node.child_by_field_name("declarator") {
            return Self::is_function_declarator(inner);
        }
        false
    }

    /// Check if the declaration has a _Thread_local or __thread storage class.
    /// tree-sitter may parse these as various node types depending on grammar version,
    /// so also do a text check on the full declaration.
    fn is_thread_local(decl_node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = decl_node.walk();
        for child in decl_node.children(&mut cursor) {
            let text = child.utf8_text(source).unwrap_or("");
            if text == "_Thread_local" || text == "__thread" || text == "thread_local" {
                return true;
            }
        }
        // Fallback: check full declaration text
        let full = decl_node.utf8_text(source).unwrap_or("");
        full.contains("_Thread_local") || full.contains("__thread") || full.contains("thread_local")
    }
}

impl GraphPipeline for GlobalMutableStatePipeline {
    fn name(&self) -> &str {
        "global_mutable_state"
    }

    fn description(&self) -> &str {
        "Detects file-scope non-const mutable variables that create hidden shared state"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.decl_query, tree.root_node(), source);

        let decl_idx = find_capture_index(&self.decl_query, "decl");

        while let Some(m) = matches.next() {
            let decl_cap = m.captures.iter().find(|c| c.index as usize == decl_idx);

            if let Some(decl_cap) = decl_cap {
                let decl_node = decl_cap.node;

                // Must be at translation_unit scope (file-level)
                if let Some(parent) = decl_node.parent() {
                    if parent.kind() != "translation_unit" {
                        continue;
                    }
                } else {
                    continue;
                }

                // Skip const declarations
                if has_type_qualifier(decl_node, source, "const") {
                    continue;
                }

                // Skip extern declarations
                if has_storage_class(decl_node, source, "extern") {
                    continue;
                }

                // Skip _Thread_local / __thread (thread-safe, not shared)
                if Self::is_thread_local(decl_node, source) {
                    continue;
                }

                // Skip function prototypes
                if let Some(declarator) = decl_node.child_by_field_name("declarator")
                    && Self::is_function_declarator(declarator)
                {
                    continue;
                }

                if is_nolint_suppressed(source, decl_node, self.name()) {
                    continue;
                }

                let var_name = decl_node
                    .child_by_field_name("declarator")
                    .and_then(|d| find_identifier_in_declarator(d, source))
                    .unwrap_or_default();

                let is_static = has_storage_class(decl_node, source, "static");

                let (severity, pattern, message) = if is_static {
                    (
                        "info",
                        "file_scoped_mutable_state",
                        format!(
                            "file-scoped mutable variable `{var_name}` — consider making const if not mutated"
                        ),
                    )
                } else {
                    (
                        "error",
                        "global_mutable_state",
                        format!(
                            "global mutable variable `{var_name}` — exposed to other translation units, consider encapsulating or making const"
                        ),
                    )
                };

                let start = decl_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: pattern.to_string(),
                    message,
                    snippet: extract_snippet(source, decl_node, 1),
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
        let pipeline = GlobalMutableStatePipeline::new().unwrap();
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
    fn detects_global_mutable() {
        let src = "int global_count = 0;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "global_mutable_state");
        assert!(findings[0].message.contains("global_count"));
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn skips_const() {
        let src = "const int MAX = 100;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_extern() {
        let src = "extern int shared_count;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_function_prototype() {
        let src = "int main(int argc, char **argv);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_local_variable() {
        let src = "void f() { int local = 0; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn static_variable_lower_severity() {
        let src = "static int count = 0;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "file_scoped_mutable_state");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn non_static_is_error_severity() {
        let src = "int global_flag = 0;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
        assert_eq!(findings[0].pattern, "global_mutable_state");
    }

    #[test]
    fn skips_thread_local() {
        let src = "_Thread_local int tls_var = 0;";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_global_array() {
        let src = "int lookup_table[256] = {0};";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn nolint_suppresses() {
        let src = "int global_count = 0; // NOLINT";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
