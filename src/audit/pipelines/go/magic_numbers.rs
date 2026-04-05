use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{COMMON_ALLOWED_NUMBERS, is_generated_go_file, is_nolint_suppressed, is_test_file};

use super::primitives::{compile_numeric_literal_query, find_capture_index};

const EXCLUDED_VALUES: &[&str] = &[
    "0", "1", "2", "0.0", "1.0", "10", "100", "1000", "256", "512", "1024", "2048", "4096", "8192",
    "16384", "32768", "65536", "0xFF", "0xff", "0x80", "0xFFFF", "0xffff",
];

const EXEMPT_ANCESTOR_KINDS: &[&str] = &[
    "const_declaration",
    "const_spec",
    "case_clause",
    "expression_case",
];

/// Built-in Go functions where numeric arguments are expected and not magic.
const SAFE_CALL_TARGETS: &[&str] = &["make", "append", "cap", "len", "new", "delete", "copy"];

/// Package names whose functions commonly take numeric formatting args.
const SAFE_CALL_PACKAGES: &[&str] = &["fmt", "log"];

pub struct GoMagicNumbersPipeline {
    numeric_query: Arc<Query>,
}

impl GoMagicNumbersPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            numeric_query: compile_numeric_literal_query()?,
        })
    }

    fn is_exempt_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if EXEMPT_ANCESTOR_KINDS.contains(&parent.kind()) {
                return true;
            }
            // Refined call_expression check: only exempt known safe targets
            if parent.kind() == "call_expression" {
                if Self::is_safe_call(parent, source) {
                    return true;
                }
            }
            current = parent.parent();
        }

        // Skip if this is an index expression
        if let Some(parent) = node.parent()
            && parent.kind() == "index_expression"
            && let Some(index_child) = parent.named_child(1)
            && index_child.id() == node.id()
        {
            return true;
        }

        // Skip well-named variable assignments
        if Self::is_well_named_assignment(node, source) {
            return true;
        }

        false
    }

    /// Check if a call_expression targets a known safe function (builtins or fmt/log packages).
    fn is_safe_call(call_node: tree_sitter::Node, source: &[u8]) -> bool {
        // The function part is the first child of call_expression
        if let Some(func) = call_node.named_child(0) {
            let func_text = func.utf8_text(source).unwrap_or("");
            // Direct builtin calls: make(...), append(...), etc.
            if SAFE_CALL_TARGETS.contains(&func_text) {
                return true;
            }
            // Package-qualified calls: fmt.Sprintf(...), log.Printf(...), etc.
            if func.kind() == "selector_expression" {
                if let Some(operand) = func.named_child(0) {
                    let pkg = operand.utf8_text(source).unwrap_or("");
                    if SAFE_CALL_PACKAGES.contains(&pkg) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Skip findings when the numeric literal is assigned to a well-named variable
    /// (more than 3 characters) via short_var_declaration.
    fn is_well_named_assignment(node: tree_sitter::Node, source: &[u8]) -> bool {
        if let Some(parent) = node.parent() {
            // Direct RHS of short_var_declaration: `name := 42`
            if parent.kind() == "short_var_declaration" {
                if let Some(left) = parent.child_by_field_name("left") {
                    // expression_list — get first identifier
                    if let Some(first_id) = left.named_child(0) {
                        let name = first_id.utf8_text(source).unwrap_or("");
                        if name.len() > 3 && name != "_" {
                            return true;
                        }
                    }
                }
            }
            // Also check var_spec: `var name = 42`
            if parent.kind() == "var_spec" {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let name = name_node.utf8_text(source).unwrap_or("");
                    if name.len() > 3 && name != "_" {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl GraphPipeline for GoMagicNumbersPipeline {
    fn name(&self) -> &str {
        "magic_numbers"
    }

    fn description(&self) -> &str {
        "Detects numeric literals outside const contexts that should be named constants"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);

        if is_test_file(file_path) {
            return Vec::new();
        }

        if is_generated_go_file(file_path, source) {
            return vec![];
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.numeric_query, tree.root_node(), source);

        let number_idx = find_capture_index(&self.numeric_query, "number");

        while let Some(m) = matches.next() {
            let num_cap = m.captures.iter().find(|c| c.index as usize == number_idx);

            if let Some(num_cap) = num_cap {
                let value = num_cap.node.utf8_text(source).unwrap_or("");

                if EXCLUDED_VALUES.contains(&value) || COMMON_ALLOWED_NUMBERS.contains(&value) {
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
                        "magic number `{value}` — consider extracting to a named constant for clarity"
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_file(source, "test.go")
    }

    fn parse_and_check_file(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Go.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = GoMagicNumbersPipeline::new().unwrap();
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
    fn detects_magic_number() {
        let src = "package main\nfunc main() {\n\tx := 42 + y\n\t_ = x\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "magic_number");
        assert!(findings[0].message.contains("42"));
    }

    #[test]
    fn skips_const_context() {
        let src = "package main\nconst maxWorkers = 32\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_common_values() {
        let src = "package main\nfunc main() {\n\tx := 1\n\ty := 0\n\tz := 2\n\t_ = x + y + z\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_index_expression() {
        let src = "package main\nfunc main() {\n\tx := arr[0]\n\t_ = x\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_float_magic_number() {
        let src = "package main\nfunc main() {\n\tpi := 3.14159\n\t_ = pi\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3.14159"));
    }

    #[test]
    fn nolint_suppression_skips_finding() {
        let src = "package main\nfunc f() {\n\tx := 42 + y // NOLINT(magic_numbers)\n\t_ = x\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn generated_file_skipped() {
        let src = "package main\nfunc f() {\n\tx := 42 + y\n\t_ = x\n}\n";
        let findings = parse_and_check_file(src, "types.pb.go");
        assert!(findings.is_empty());
    }

    #[test]
    fn make_call_not_flagged() {
        let src = "package main\nfunc f() {\n\ts := make([]byte, 4096)\n\t_ = s\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn well_named_variable_skip() {
        let src = "package main\nfunc f() {\n\tmaxRetries := 3\n\t_ = maxRetries\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
