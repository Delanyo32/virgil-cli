use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};

use super::primitives::{
    compile_function_definition_query, extract_snippet, find_capture_index,
    find_identifier_in_declarator, node_text,
};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

const ALLOC_FUNCTIONS: &[&str] = &[
    "malloc",
    "calloc",
    "realloc",
    "strdup",
    "strndup",
    "asprintf",
    "aligned_alloc",
    "reallocarray",
];

/// Function name substrings that suggest ownership transfer.
const OWNERSHIP_TRANSFER_HINTS: &[&str] = &[
    "set_", "add_", "push_", "insert_", "append_", "register_", "attach_", "enqueue_",
    "_set", "_add", "_push", "_insert", "_append", "_register", "_attach",
];

pub struct MemoryLeaksPipeline {
    fn_def_query: Arc<Query>,
}

impl MemoryLeaksPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            fn_def_query: compile_function_definition_query()?,
        })
    }

    /// Scan a function body and return:
    /// - allocations: (node, var_name)
    /// - freed_vars: set of variable names passed to free()
    /// - returned_vars: variable names that appear in return statements
    /// - struct_stored_vars: variable names assigned to struct fields
    /// - transferred_vars: variable names passed to ownership-transfer functions
    /// - goto_freed_vars: variable names freed in labeled cleanup blocks
    fn scan_body<'a>(
        body: tree_sitter::Node<'a>,
        source: &[u8],
    ) -> ScanResult<'a> {
        let mut result = ScanResult::default();
        Self::walk_body(body, source, &mut result);
        result
    }

    fn walk_body<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        result: &mut ScanResult<'a>,
    ) {
        // Check for allocation in declarations
        if node.kind() == "declaration"
            && let Some(declarator) = node.child_by_field_name("declarator")
            && declarator.kind() == "init_declarator"
            && let Some(value) = declarator.child_by_field_name("value")
            && Self::is_alloc_call(value, source)
            && let Some(decl) = declarator.child_by_field_name("declarator")
            && let Some(var_name) = find_identifier_in_declarator(decl, source)
        {
            result.allocs.push((node, var_name));
        }

        // Check for free() calls — extract the argument name
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
            && node_text(func, source) == "free"
            && let Some(args) = node.child_by_field_name("arguments")
        {
            let mut cursor = args.walk();
            for arg in args.named_children(&mut cursor) {
                if arg.kind() == "identifier" {
                    result.freed_vars.insert(node_text(arg, source).to_string());
                }
            }
        }

        // Check for return statements
        if node.kind() == "return_statement" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    result.returned_vars.insert(node_text(child, source).to_string());
                }
            }
        }

        // Check for struct field assignment: ctx->buf = ptr; or s.buf = ptr;
        if node.kind() == "assignment_expression"
            && let Some(left) = node.child_by_field_name("left")
            && left.kind() == "field_expression"
            && let Some(right) = node.child_by_field_name("right")
            && right.kind() == "identifier"
        {
            result
                .struct_stored_vars
                .insert(node_text(right, source).to_string());
        }

        // Check for calls that might transfer ownership
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
        {
            let fn_name = node_text(func, source);
            if fn_name != "free"
                && OWNERSHIP_TRANSFER_HINTS.iter().any(|h| fn_name.contains(h))
                && let Some(args) = node.child_by_field_name("arguments")
            {
                let mut cursor = args.walk();
                for arg in args.named_children(&mut cursor) {
                    if arg.kind() == "identifier" {
                        result
                            .transferred_vars
                            .insert(node_text(arg, source).to_string());
                    }
                }
            }
        }

        // Check for labeled statements containing free() — goto cleanup pattern
        if node.kind() == "labeled_statement" {
            // Scan the labeled block for free() calls
            Self::scan_label_for_frees(node, source, &mut result.goto_freed_vars);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_body(child, source, result);
        }
    }

    fn scan_label_for_frees(
        node: tree_sitter::Node,
        source: &[u8],
        freed: &mut HashSet<String>,
    ) {
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
            && node_text(func, source) == "free"
            && let Some(args) = node.child_by_field_name("arguments")
        {
            let mut cursor = args.walk();
            for arg in args.named_children(&mut cursor) {
                if arg.kind() == "identifier" {
                    freed.insert(node_text(arg, source).to_string());
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::scan_label_for_frees(child, source, freed);
        }
    }

    fn is_alloc_call(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "call_expression"
            && let Some(func) = node.child_by_field_name("function")
        {
            let fn_name = node_text(func, source);
            return ALLOC_FUNCTIONS.contains(&fn_name);
        }
        if node.kind() == "cast_expression"
            && let Some(value) = node.child_by_field_name("value")
        {
            return Self::is_alloc_call(value, source);
        }
        false
    }
}

#[derive(Default)]
struct ScanResult<'a> {
    allocs: Vec<(tree_sitter::Node<'a>, String)>,
    freed_vars: HashSet<String>,
    returned_vars: HashSet<String>,
    struct_stored_vars: HashSet<String>,
    transferred_vars: HashSet<String>,
    goto_freed_vars: HashSet<String>,
}

impl GraphPipeline for MemoryLeaksPipeline {
    fn name(&self) -> &str {
        "memory_leaks"
    }

    fn description(&self) -> &str {
        "Detects malloc/calloc allocations without corresponding free in the same function"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let fn_body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_cap = m.captures.iter().find(|c| c.index as usize == fn_body_idx);

            if let Some(body_cap) = body_cap {
                let result = Self::scan_body(body_cap.node, source);

                if result.allocs.is_empty() {
                    continue;
                }

                for (alloc_node, var_name) in &result.allocs {
                    // Skip if freed
                    if result.freed_vars.contains(var_name) {
                        continue;
                    }
                    // Skip if freed via goto cleanup label
                    if result.goto_freed_vars.contains(var_name) {
                        continue;
                    }
                    // Skip if returned
                    if result.returned_vars.contains(var_name) {
                        continue;
                    }
                    // Skip if stored in struct field (ownership transferred to struct)
                    if result.struct_stored_vars.contains(var_name) {
                        continue;
                    }
                    // Skip if passed to ownership-transfer function
                    if result.transferred_vars.contains(var_name) {
                        continue;
                    }

                    if is_nolint_suppressed(source, *alloc_node, self.name()) {
                        continue;
                    }

                    let start = alloc_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "potential_memory_leak".to_string(),
                        message: format!(
                            "`{var_name}` is allocated but never freed in this function"
                        ),
                        snippet: extract_snippet(source, *alloc_node, 1),
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
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeaksPipeline::new().unwrap();
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
    fn detects_missing_free() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10);
    p[0] = 1;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "potential_memory_leak");
        assert!(findings[0].message.contains("p"));
    }

    #[test]
    fn skips_with_free() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10);
    p[0] = 1;
    free(p);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_returned_pointer() {
        let src = r#"
int *create() {
    int *p = malloc(sizeof(int));
    return p;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn multiple_allocs_one_freed() {
        // The broken has_free bug: previously one free() for ANY var suppressed ALL leaks
        let src = r#"
void f() {
    int *p = malloc(10);
    int *q = malloc(20);
    free(q);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("p"));
    }

    #[test]
    fn detects_strdup() {
        let src = r#"
void f(const char *input) {
    char *s = strdup(input);
    s[0] = 'x';
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_realloc() {
        let src = r#"
void f() {
    int *p = realloc(0, 100);
    p[0] = 1;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_goto_cleanup() {
        let src = r#"
int f() {
    int *p = malloc(100);
    if (!p) return -1;
    if (error) goto cleanup;
    return 0;
cleanup:
    free(p);
    return -1;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_struct_ownership_transfer() {
        let src = r#"
void f(struct Ctx *ctx) {
    char *buf = malloc(1024);
    ctx->buffer = buf;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_ownership_transfer_function() {
        let src = r#"
void f(struct List *list) {
    int *item = malloc(sizeof(int));
    list_append(list, item);
}
"#;
        // list_append contains "append_" pattern — treat as ownership transfer
        // Actually let me check: the hint is "_append" not "append_"
        // list_append matches "_append" pattern — wait, it has "append" not "_append"
        // But OWNERSHIP_TRANSFER_HINTS has "append_" and "_append"
        // "list_append" contains "_append" — no, it doesn't. It's "list_append" which contains "append"
        // Hmm, let me reconsider. "list_append" does not contain "_append" literally.
        // But it contains "append_" if there were a trailing underscore. It doesn't.
        // Actually neither "append_" nor "_append" is a substring of "list_append"
        // "_append" IS a substring: "list_append" → "list" + "_append"
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses() {
        let src = r#"
void f() {
    int *p = malloc(sizeof(int) * 10); // NOLINT
    p[0] = 1;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
