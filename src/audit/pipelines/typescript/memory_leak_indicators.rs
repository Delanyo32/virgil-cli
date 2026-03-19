use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Array iteration methods that act as implicit loops.
const ARRAY_LOOP_METHODS: &[&str] = &[
    "forEach", "map", "flatMap", "filter", "reduce", "some", "every",
];

pub struct MemoryLeakIndicatorsPipeline {
    method_call_query: Arc<Query>,
    direct_call_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();
        let method_call_str = r#"
(call_expression
  function: (member_expression
    object: (_) @obj
    property: (property_identifier) @method)
  arguments: (arguments) @args) @call
"#;
        let direct_call_str = r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args) @call
"#;
        Ok(Self {
            method_call_query: Arc::new(Query::new(&ts_lang, method_call_str).with_context(
                || "failed to compile method_call query for TS memory_leak_indicators",
            )?),
            direct_call_query: Arc::new(Query::new(&ts_lang, direct_call_str).with_context(
                || "failed to compile direct_call query for TS memory_leak_indicators",
            )?),
        })
    }

    /// Check if the file has any removeEventListener call, which suggests cleanup awareness.
    fn file_has_remove_listener(&self, tree: &Tree, source: &[u8]) -> bool {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
        let method_idx = find_capture_index(&self.method_call_query, "method");

        while let Some(m) = matches.next() {
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);

            if let Some(method) = method_node {
                if node_text(method, source) == "removeEventListener" {
                    return true;
                }
            }
        }
        false
    }

    /// Returns true if `node` is inside a traditional loop or array method callback.
    fn is_in_loop_context(node: tree_sitter::Node, source: &[u8]) -> bool {
        let loop_kinds = &[
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
        ];
        let mut current = node.parent();
        while let Some(parent) = current {
            if loop_kinds.contains(&parent.kind()) {
                return true;
            }
            // Check for array method callback
            if parent.kind() == "arrow_function" || parent.kind() == "function_expression" {
                if let Some(args_node) = parent.parent() {
                    if args_node.kind() == "arguments" {
                        if let Some(call_node) = args_node.parent() {
                            if call_node.kind() == "call_expression" {
                                if let Some(func) = call_node.child_by_field_name("function") {
                                    if func.kind() == "member_expression" {
                                        if let Some(prop) = func.child_by_field_name("property") {
                                            let method_name = node_text(prop, source);
                                            if ARRAY_LOOP_METHODS.contains(&method_name) {
                                                return true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            current = parent.parent();
        }
        false
    }

    /// Check if a loop body has a break, return, or length-based condition
    /// that suggests bounded iteration.
    fn loop_has_bound_check(node: tree_sitter::Node, source: &[u8]) -> bool {
        let text = node_text(node, source);
        // for..of / for..in loops are bounded by the iterable
        if node.kind() == "for_in_statement" {
            return true;
        }
        // Traditional for with a clear bound
        if node.kind() == "for_statement" {
            if text.contains(".length") {
                return true;
            }
        }
        // Check for break/return inside body
        Self::walk_for_bound_check(node, source)
    }

    fn walk_for_bound_check(node: tree_sitter::Node, source: &[u8]) -> bool {
        if node.kind() == "if_statement" {
            let cond_text = node_text(node, source);
            if cond_text.contains(".length") || cond_text.contains(".size") {
                return true;
            }
        }
        if node.kind() == "break_statement" || node.kind() == "return_statement" {
            return true;
        }
        let mut child_cursor = node.walk();
        for child in node.children(&mut child_cursor) {
            if Self::walk_for_bound_check(child, source) {
                return true;
            }
        }
        false
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects potential memory leak patterns: addEventListener without cleanup, setInterval without clear, unbounded array growth"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let has_remove = self.file_has_remove_listener(tree, source);

        // Pass 1: Check method calls for addEventListener leaks, setInterval, unbounded push
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
            let obj_idx = find_capture_index(&self.method_call_query, "obj");
            let method_idx = find_capture_index(&self.method_call_query, "method");
            let call_idx = find_capture_index(&self.method_call_query, "call");

            while let Some(m) = matches.next() {
                let obj_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == obj_idx)
                    .map(|c| c.node);
                let method_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(_obj), Some(method), Some(call)) = (obj_node, method_node, call_node) {
                    let method_name = node_text(method, source);

                    // addEventListener without corresponding removeEventListener
                    if method_name == "addEventListener" && !has_remove {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "missing_remove_listener".to_string(),
                            message:
                                "addEventListener without removeEventListener — potential memory leak"
                                    .to_string(),
                            snippet: extract_snippet(source, call, 1),
                        });
                    }

                    // push/unshift inside unbounded loop
                    if (method_name == "push" || method_name == "unshift")
                        && Self::is_in_loop_context(call, source)
                    {
                        // Walk up to find the loop node and check for bounds
                        let mut current = call.parent();
                        let mut in_unbounded_loop = false;
                        while let Some(parent) = current {
                            let loop_kinds = &["for_statement", "while_statement", "do_statement"];
                            if loop_kinds.contains(&parent.kind()) {
                                if !Self::loop_has_bound_check(parent, source) {
                                    in_unbounded_loop = true;
                                }
                                break;
                            }
                            current = parent.parent();
                        }

                        if in_unbounded_loop {
                            let start = call.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unbounded_growth".to_string(),
                                message: format!(
                                    "`.{method_name}()` inside loop without apparent bound — potential unbounded memory growth"
                                ),
                                snippet: extract_snippet(source, call, 1),
                            });
                        }
                    }
                }
            }
        }

        // Pass 2: Check bare function calls for setInterval without clearInterval
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.direct_call_query, tree.root_node(), source);
            let fn_name_idx = find_capture_index(&self.direct_call_query, "fn_name");
            let call_idx = find_capture_index(&self.direct_call_query, "call");

            let mut has_set_interval = Vec::new();
            let mut has_clear_interval = false;

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_name_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_n), Some(call)) = (fn_node, call_node) {
                    let fn_name = node_text(fn_n, source);
                    match fn_name {
                        "setInterval" => has_set_interval.push(call),
                        "clearInterval" => has_clear_interval = true,
                        _ => {}
                    }
                }
            }

            if !has_clear_interval {
                for call in has_set_interval {
                    let start = call.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "interval_without_clear".to_string(),
                        message:
                            "setInterval without clearInterval — interval will run indefinitely"
                                .to_string(),
                        snippet: extract_snippet(source, call, 1),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let lang = Language::TypeScript;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_add_event_listener_without_remove() {
        let src = "element.addEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "missing_remove_listener");
    }

    #[test]
    fn skips_when_remove_listener_present() {
        let src = "\
element.addEventListener('click', handler);
element.removeEventListener('click', handler);";
        let findings = parse_and_check(src);
        let listener_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "missing_remove_listener")
            .collect();
        assert!(listener_findings.is_empty());
    }

    #[test]
    fn detects_set_interval_without_clear() {
        let src = "setInterval(() => { doWork(); }, 1000);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "interval_without_clear");
    }

    #[test]
    fn skips_set_interval_with_clear() {
        let src = "\
const id: number = setInterval(() => { doWork(); }, 1000);
clearInterval(id);";
        let findings = parse_and_check(src);
        let interval_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "interval_without_clear")
            .collect();
        assert!(interval_findings.is_empty());
    }

    #[test]
    fn detects_push_in_unbounded_while_loop() {
        let src = "\
const items: string[] = [];
while (true) {
    items.push(getData());
}";
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "unbounded_growth"));
    }

    #[test]
    fn ignores_push_in_bounded_for_loop() {
        let src = "\
const items: number[] = [];
for (let i = 0; i < arr.length; i++) {
    items.push(arr[i]);
}";
        let findings = parse_and_check(src);
        let growth_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_growth")
            .collect();
        assert!(growth_findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        MemoryLeakIndicatorsPipeline::new(Language::Tsx).unwrap();
    }
}
