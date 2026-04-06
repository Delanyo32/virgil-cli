use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_call_expression_query, extract_snippet, find_capture_index, node_text,
};

pub struct EventListenerLeakPipeline {
    call_query: Arc<Query>,
}

impl EventListenerLeakPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            call_query: compile_call_expression_query()?,
        })
    }
}

/// Extract the first argument text (event type) and second argument text (handler) from a call node.
fn extract_listener_args<'a>(call_node: Node<'a>, source: &'a [u8]) -> Option<(String, String)> {
    let args_node = call_node.child_by_field_name("arguments")?;
    let mut arg_index = 0;
    let mut event_type = None;
    let mut handler = None;

    for i in 0..args_node.named_child_count() {
        let child = args_node.named_child(i)?;
        match arg_index {
            0 => event_type = Some(node_text(child, source).to_string()),
            1 => handler = Some(node_text(child, source).to_string()),
            _ => break,
        }
        arg_index += 1;
    }

    Some((event_type?, handler?))
}

/// Check if the handler argument (2nd arg) is an anonymous function (arrow or function expression).
fn is_anonymous_handler(call_node: Node, source: &[u8]) -> bool {
    let args_node = match call_node.child_by_field_name("arguments") {
        Some(n) => n,
        None => return false,
    };

    // Get second named child (handler argument)
    if args_node.named_child_count() < 2 {
        return false;
    }
    let handler_node = match args_node.named_child(1) {
        Some(n) => n,
        None => return false,
    };

    let kind = handler_node.kind();
    // Direct anonymous function/arrow
    if kind == "arrow_function" || kind == "function_expression" {
        return true;
    }
    // Also check for inline function: .bind() calls wrapping a function
    if kind == "call_expression" {
        let text = node_text(handler_node, source);
        if text.contains(".bind(") {
            return true;
        }
    }
    false
}

/// Check if the third argument contains `{ once: true }`.
fn has_once_option(call_node: Node, source: &[u8]) -> bool {
    let args_node = match call_node.child_by_field_name("arguments") {
        Some(n) => n,
        None => return false,
    };

    if args_node.named_child_count() < 3 {
        return false;
    }

    let options_node = match args_node.named_child(2) {
        Some(n) => n,
        None => return false,
    };

    if options_node.kind() != "object" {
        return false;
    }

    // Walk object properties looking for `once: true`
    for i in 0..options_node.named_child_count() {
        if let Some(prop) = options_node.named_child(i) {
            if prop.kind() == "pair" {
                let key = prop.child_by_field_name("key");
                let value = prop.child_by_field_name("value");
                if let (Some(k), Some(v)) = (key, value) {
                    if node_text(k, source) == "once" && node_text(v, source) == "true" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Check if the third argument contains `{ signal: ... }` (AbortController pattern).
fn has_signal_option(call_node: Node, source: &[u8]) -> bool {
    let args_node = match call_node.child_by_field_name("arguments") {
        Some(n) => n,
        None => return false,
    };

    if args_node.named_child_count() < 3 {
        return false;
    }

    let options_node = match args_node.named_child(2) {
        Some(n) => n,
        None => return false,
    };

    if options_node.kind() != "object" {
        return false;
    }

    for i in 0..options_node.named_child_count() {
        if let Some(prop) = options_node.named_child(i) {
            if prop.kind() == "pair" {
                if let Some(k) = prop.child_by_field_name("key") {
                    if node_text(k, source) == "signal" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

impl GraphPipeline for EventListenerLeakPipeline {
    fn name(&self) -> &str {
        "event_listener_leak"
    }

    fn description(&self) -> &str {
        "Detects addEventListener calls without corresponding removeEventListener"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree: &Tree = ctx.tree;
        let source: &[u8] = ctx.source;
        let file_path: &str = ctx.file_path;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let method_idx = find_capture_index(&self.call_query, "method");
        let call_idx = find_capture_index(&self.call_query, "call");

        // Collect add/remove calls with their event type + handler text
        let mut add_calls: Vec<(Node, String, String)> = Vec::new(); // (node, event_type, handler)
        let mut remove_keys: HashMap<(String, String), bool> = HashMap::new(); // (event_type, handler) -> exists

        while let Some(m) = matches.next() {
            let method_node = m.captures.iter().find(|c| c.index as usize == method_idx);
            let call_node = m.captures.iter().find(|c| c.index as usize == call_idx);

            if let (Some(method), Some(call)) = (method_node, call_node) {
                let method_name = node_text(method.node, source);
                match method_name {
                    "addEventListener" => {
                        if let Some((event_type, handler)) =
                            extract_listener_args(call.node, source)
                        {
                            add_calls.push((call.node, event_type, handler));
                        } else {
                            // Could not extract args; still flag it
                            add_calls.push((
                                call.node,
                                String::new(),
                                String::new(),
                            ));
                        }
                    }
                    "removeEventListener" => {
                        if let Some((event_type, handler)) =
                            extract_listener_args(call.node, source)
                        {
                            remove_keys.insert((event_type, handler), true);
                        }
                    }
                    _ => {}
                }
            }
        }

        if add_calls.is_empty() {
            return Vec::new();
        }

        let mut findings = Vec::new();

        for (node, event_type, handler) in &add_calls {
            // NOLINT suppression
            if is_nolint_suppressed(source, *node, self.name()) {
                continue;
            }

            // { once: true } auto-removes -- no leak
            if has_once_option(*node, source) {
                continue;
            }

            // { signal: controller.signal } -- cleanup via abort
            if has_signal_option(*node, source) {
                continue;
            }

            // Check if a matching removeEventListener exists
            let key = (event_type.clone(), handler.clone());
            if !event_type.is_empty() && !handler.is_empty() && remove_keys.contains_key(&key) {
                continue;
            }

            let start = node.start_position();

            // Detect anonymous handler -- can never be properly removed
            if is_anonymous_handler(*node, source) {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "anonymous_listener_leak".to_string(),
                    message: "addEventListener with anonymous handler cannot be removed — use a named function".to_string(),
                    snippet: extract_snippet(source, *node, 1),
                });
            } else {
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "missing_remove_listener".to_string(),
                    message: "addEventListener without matching removeEventListener — potential memory leak".to_string(),
                    snippet: extract_snippet(source, *node, 1),
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
        parse_and_check_path(source, "test.js")
    }

    fn parse_and_check_path(source: &str, file_path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = EventListenerLeakPipeline::new().unwrap();
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
    fn detects_add_without_remove() {
        let src = "element.addEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_remove_listener");
    }

    #[test]
    fn skips_when_matching_remove_present() {
        let src = "element.addEventListener('click', handler);\nelement.removeEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_findings_without_add() {
        let src = "element.removeEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_unmatched_when_different_event_type() {
        let src = "element.addEventListener('click', handler);\nelement.removeEventListener('keyup', handler);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_remove_listener");
    }

    #[test]
    fn flags_unmatched_when_different_handler() {
        let src = "element.addEventListener('click', handlerA);\nelement.removeEventListener('click', handlerB);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn multiple_add_one_unrelated_remove() {
        let src = r#"
element.addEventListener('click', onClick);
element.addEventListener('keyup', onKeyup);
element.addEventListener('scroll', onScroll);
element.removeEventListener('click', onClick);
"#;
        let findings = parse_and_check(src);
        // click is matched; keyup and scroll are not
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn suppresses_once_option() {
        let src = "element.addEventListener('click', handler, { once: true });";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn suppresses_signal_option() {
        let src = "element.addEventListener('click', handler, { signal: controller.signal });";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_anonymous_arrow_handler() {
        let src = "element.addEventListener('click', () => { console.log('clicked'); });";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "anonymous_listener_leak");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn detects_anonymous_function_expression() {
        let src = "element.addEventListener('click', function() { console.log('clicked'); });";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "anonymous_listener_leak");
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "// NOLINT(event_listener_leak)\nelement.addEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
