use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{ancestor_has_kind, is_generated_go_file, is_nolint_suppressed};

use super::primitives::{
    compile_assignment_query, compile_short_var_decl_query, extract_snippet, find_capture_index,
    node_text,
};

const SAFE_CLEANUP_METHODS: &[&str] = &[
    "Close",
    "Flush",
    "Remove",
    "Sync",
    "Reset",
    "Stop",
    "Shutdown",
    "Unsubscribe",
    "SetDeadline",
    "SetReadDeadline",
    "SetWriteDeadline",
];

pub struct ErrorSwallowingPipeline {
    short_var_query: Arc<Query>,
    assign_query: Arc<Query>,
}

impl ErrorSwallowingPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            short_var_query: compile_short_var_decl_query()?,
            assign_query: compile_assignment_query()?,
        })
    }

    /// Determine severity based on the called function/package.
    fn classify_severity(rhs: tree_sitter::Node, source: &[u8]) -> &'static str {
        // Look for call_expression children in the RHS
        for i in 0..rhs.named_child_count() {
            if let Some(child) = rhs.named_child(i) {
                if child.kind() == "call_expression" {
                    if let Some(func) = child.child_by_field_name("function") {
                        if func.kind() == "selector_expression" {
                            let pkg = func
                                .child_by_field_name("operand")
                                .map(|n| node_text(n, source))
                                .unwrap_or("");
                            let method = func
                                .child_by_field_name("field")
                                .map(|n| node_text(n, source))
                                .unwrap_or("");

                            // I/O / network calls → error
                            let io_packages = ["os", "sql", "net", "http"];
                            let io_methods = ["Open", "Create", "Dial", "Listen", "Connect"];
                            if io_packages.contains(&pkg) || io_methods.contains(&method) {
                                return "error";
                            }

                            // Logging / formatting calls → info
                            let log_packages = ["fmt", "log"];
                            let log_methods = ["Println", "Printf", "Fprintf", "Print"];
                            if log_packages.contains(&pkg) || log_methods.contains(&method) {
                                return "info";
                            }
                        } else {
                            // Plain function call (no selector) — check name for known patterns
                            let func_name = node_text(func, source);
                            let io_names = ["Open", "Create", "Dial", "Listen", "Connect"];
                            if io_names.contains(&func_name) {
                                return "error";
                            }
                            let log_names = ["Println", "Printf", "Fprintf", "Print"];
                            if log_names.contains(&func_name) {
                                return "info";
                            }
                        }
                    }
                }
            }
        }
        "warning"
    }

    fn check_declaration(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        query: &Query,
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        let lhs_idx = find_capture_index(query, "lhs");
        let rhs_idx = find_capture_index(query, "rhs");
        let decl_idx = query
            .capture_names()
            .iter()
            .position(|n| *n == "decl" || *n == "assign")
            .expect("query must have @decl or @assign capture");

        while let Some(m) = matches.next() {
            let lhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == lhs_idx)
                .map(|c| c.node);
            let rhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == rhs_idx)
                .map(|c| c.node);
            let decl_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == decl_idx)
                .map(|c| c.node);

            if let (Some(lhs), Some(rhs), Some(decl)) = (lhs_node, rhs_node, decl_node) {
                // Check if any LHS element is blank identifier `_`
                let has_blank = (0..lhs.named_child_count()).any(|i| {
                    lhs.named_child(i)
                        .map(|child| {
                            child.kind() == "identifier" && node_text(child, source) == "_"
                        })
                        .unwrap_or(false)
                });

                if !has_blank {
                    continue;
                }

                // Verify RHS contains a call_expression (not map access, type assertion, etc.)
                let has_call = (0..rhs.named_child_count()).any(|i| {
                    rhs.named_child(i)
                        .map(|child| child.kind() == "call_expression")
                        .unwrap_or(false)
                });

                if !has_call {
                    continue;
                }

                // Skip if the call is inside a defer statement
                if ancestor_has_kind(decl, &["defer_statement"]) {
                    continue;
                }

                // Skip if the RHS call is to a known safe-to-ignore cleanup function
                let is_safe_cleanup = (0..rhs.named_child_count()).any(|i| {
                    rhs.named_child(i)
                        .map(|child| {
                            if child.kind() == "call_expression" {
                                // Check for selector_expression (e.g., file.Close())
                                if let Some(func) = child.child_by_field_name("function")
                                    && func.kind() == "selector_expression"
                                    && let Some(field) = func.child_by_field_name("field")
                                {
                                    let method = node_text(field, source);
                                    return SAFE_CLEANUP_METHODS.contains(&method);
                                }
                            }
                            false
                        })
                        .unwrap_or(false)
                });

                if is_safe_cleanup {
                    continue;
                }

                // Skip if suppressed by NOLINT comment
                if is_nolint_suppressed(source, decl, self.name()) {
                    continue;
                }

                let severity = Self::classify_severity(rhs, source);
                let start = decl.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "error_swallowed".to_string(),
                    message: "error return value discarded with blank identifier `_`".to_string(),
                    snippet: extract_snippet(source, decl, 1),
                });
            }
        }

        findings
    }
}

impl GraphPipeline for ErrorSwallowingPipeline {
    fn name(&self) -> &str {
        "error_swallowing"
    }

    fn description(&self) -> &str {
        "Detects discarded error returns via blank identifier: `data, _ := someFunc()`"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_generated_go_file(file_path, source) {
            return vec![];
        }

        let mut findings = Vec::new();
        findings.extend(self.check_declaration(tree, source, &self.short_var_query, file_path));
        findings.extend(self.check_declaration(tree, source, &self.assign_query, file_path));
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_file(source, "test.go")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ErrorSwallowingPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_short_var_decl_error_swallow() {
        let src = "package main\nfunc main() {\n\tdata, _ := someFunc()\n\t_ = data\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "error_swallowed");
    }

    #[test]
    fn detects_assignment_error_swallow() {
        let src =
            "package main\nfunc main() {\n\tvar data int\n\tdata, _ = someFunc()\n\t_ = data\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "error_swallowed");
    }

    #[test]
    fn skips_map_access() {
        let src = "package main\nfunc main() {\n\t_, ok := myMap[key]\n\t_ = ok\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_single_blank_without_call() {
        let src = "package main\nfunc main() {\n\t_ = someValue\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_code_with_error_handling() {
        let src = "package main\nfunc main() {\n\tdata, err := someFunc()\n\tif err != nil {\n\t\treturn\n\t}\n\t_ = data\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppression_skips_finding() {
        let src = "package main\nfunc f() {\n\t_, _ = someFunc() // NOLINT(error_swallowing)\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc f() {\n\t_, _ = someFunc()\n}\n";
        let findings = parse_and_check_file(src, "model.pb.go");
        assert!(findings.is_empty());
    }

    #[test]
    fn sync_method_not_flagged() {
        let src = "package main\nfunc f() {\n\t_ = f.Sync()\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn os_open_error_severity() {
        let src = "package main\nfunc f() {\n\t_, _ = os.Open(\"file.txt\")\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn fmt_println_info_severity() {
        let src = "package main\nfunc f() {\n\t_, _ = fmt.Println(\"hello\")\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }
}
