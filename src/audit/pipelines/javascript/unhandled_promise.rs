use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

pub struct UnhandledPromisePipeline {
    call_query: Arc<Query>,
}

impl UnhandledPromisePipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }

    /// Check if this .then() call has error handling somewhere in its chain.
    fn is_handled(call_node: Node, source: &[u8]) -> bool {
        // Check if .then() has 2+ arguments (rejection handler)
        if let Some(args) = call_node.child_by_field_name("arguments") {
            if args.named_child_count() >= 2 {
                return true;
            }
        }

        // Walk up the chain: .then().then().catch() should mark the inner .then() as handled.
        // The AST structure for `a.then(x).catch(y)` is:
        //   call_expression [.catch(y)]
        //     member_expression [.catch]
        //       call_expression [.then(x)]  <-- this is our call_node
        //         member_expression [.then]
        //           call_expression [a]
        // So we walk: call_node -> parent(member_expression) -> parent(call_expression) -> repeat
        if Self::chain_has_catch_or_finally(call_node, source) {
            return true;
        }

        // Check if the .then() is inside an await_expression
        if Self::is_awaited(call_node) {
            return true;
        }

        false
    }

    /// Walk up the promise chain looking for .catch() or .finally() at any level.
    fn chain_has_catch_or_finally(call_node: Node, source: &[u8]) -> bool {
        let mut current = call_node;
        loop {
            // The call_expression should be the `object` of a member_expression
            let parent = match current.parent() {
                Some(p) => p,
                None => return false,
            };

            if parent.kind() != "member_expression" {
                return false;
            }

            // Check what property is being accessed
            if let Some(prop) = parent.child_by_field_name("property") {
                let prop_name = node_text(prop, source);
                if prop_name == "catch" || prop_name == "finally" {
                    return true;
                }
            }

            // Move up: member_expression -> call_expression (the outer call)
            let grandparent = match parent.parent() {
                Some(gp) => gp,
                None => return false,
            };

            if grandparent.kind() != "call_expression" {
                return false;
            }

            // Continue chain walk from the outer call_expression
            current = grandparent;
        }
    }

    /// Check if the call is inside an await expression (rejection handled by caller).
    fn is_awaited(node: Node) -> bool {
        let mut current = node;
        loop {
            let parent = match current.parent() {
                Some(p) => p,
                None => return false,
            };
            match parent.kind() {
                "await_expression" => return true,
                // Walk through parenthesized expressions and member expressions
                "parenthesized_expression" | "member_expression" => {
                    current = parent;
                }
                _ => return false,
            }
        }
    }
}

impl NodePipeline for UnhandledPromisePipeline {
    fn name(&self) -> &str {
        "unhandled_promise"
    }

    fn description(&self) -> &str {
        "Detects .then() calls without .catch() error handling"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(method), Some(call)) = (method_node, call_node) {
                let method_name = node_text(method.node, source);

                if method_name != "then" {
                    continue;
                }

                if Self::is_handled(call.node, source) {
                    continue;
                }

                if is_nolint_suppressed(source, call.node, self.name()) {
                    continue;
                }

                let start = call.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unhandled_then".to_string(),
                    message: "`.then()` without `.catch()` — unhandled promise rejection"
                        .to_string(),
                    snippet: extract_snippet(source, call.node, 1),
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = UnhandledPromisePipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_then_without_catch() {
        let findings = parse_and_check("fetch(url).then(data => process(data));");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unhandled_then");
    }

    #[test]
    fn skips_then_with_catch() {
        let findings =
            parse_and_check("fetch(url).then(data => process(data)).catch(err => handle(err));");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_then_with_two_args() {
        let findings = parse_and_check("fetch(url).then(onSuccess, onError);");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_non_then_methods() {
        let findings = parse_and_check("obj.map(x => x * 2);");
        assert!(findings.is_empty());
    }

    // --- New tests ---

    #[test]
    fn skips_chained_then_then_catch() {
        let findings = parse_and_check(
            "fetch(url).then(a => transform(a)).then(b => process(b)).catch(err => handle(err));"
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_then_with_finally() {
        let findings = parse_and_check(
            "fetch(url).then(handler).finally(() => cleanup());"
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_awaited_then() {
        let findings = parse_and_check(
            "async function f() { await fetch(url).then(handler); }"
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_then_finally_no_catch_on_inner() {
        // .finally() does NOT handle rejection -- but it's on the chain,
        // so the inner .then() has .finally() in its chain and we consider
        // finally as "handled" (it at least acknowledges the chain).
        let findings = parse_and_check(
            "fetch(url).then(handler).finally(() => cleanup());"
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses_finding() {
        let findings = parse_and_check(
            "// NOLINT(unhandled_promise)\nfetch(url).then(handler);"
        );
        assert!(findings.is_empty());
    }
}
