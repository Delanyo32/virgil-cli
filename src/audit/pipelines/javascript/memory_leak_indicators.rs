use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Traditional loop node kinds.
const LOOP_KINDS: &[&str] = &[
    "for_statement",
    "for_in_statement",
    "while_statement",
    "do_statement",
];

/// Array methods that grow arrays.
const ARRAY_GROWTH_METHODS: &[&str] = &["push", "unshift"];

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

pub struct MemoryLeakIndicatorsPipeline {
    method_call_query: Arc<Query>,
    direct_call_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
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
            method_call_query: Arc::new(Query::new(&js_lang(), method_call_str).with_context(
                || "failed to compile method_call query for memory_leak_indicators",
            )?),
            direct_call_query: Arc::new(Query::new(&js_lang(), direct_call_str).with_context(
                || "failed to compile direct_call query for memory_leak_indicators",
            )?),
        })
    }

    /// Returns true if `node` is inside a traditional loop body.
    fn is_inside_loop(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if LOOP_KINDS.contains(&parent.kind()) {
                return true;
            }
            current = parent.parent();
        }
        false
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects potential memory leaks: addEventListener without removeEventListener, uncleared setInterval, unbounded array growth in loops"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // --- Event listener leak detection ---
        // Collect all addEventListener and removeEventListener calls, then report
        // addEventListener calls if there is no removeEventListener anywhere in the file.
        let mut add_listener_nodes = Vec::new();
        let mut has_remove_listener = false;

        // --- setInterval without clearInterval detection ---
        // Collect setInterval calls where the return value is not stored.
        let mut set_interval_nodes = Vec::new();
        let mut has_clear_interval = false;

        // --- Unbounded array growth in loops ---
        let mut growth_in_loop_nodes = Vec::new();

        // Scan method calls
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
            let method_idx = find_capture_index(&self.method_call_query, "method");
            let call_idx = find_capture_index(&self.method_call_query, "call");

            while let Some(m) = matches.next() {
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

                if let (Some(method), Some(call)) = (method_node, call_node) {
                    let method_name = node_text(method, source);

                    match method_name {
                        "addEventListener" => {
                            add_listener_nodes.push(call);
                        }
                        "removeEventListener" => {
                            has_remove_listener = true;
                        }
                        _ => {}
                    }

                    // Check for .push() / .unshift() inside loops
                    if ARRAY_GROWTH_METHODS.contains(&method_name) && Self::is_inside_loop(call) {
                        growth_in_loop_nodes.push(call);
                    }
                }
            }
        }

        // Scan direct function calls (setInterval, clearInterval)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.direct_call_query, tree.root_node(), source);
            let fn_name_idx = find_capture_index(&self.direct_call_query, "fn_name");
            let call_idx = find_capture_index(&self.direct_call_query, "call");

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

                    if fn_name == "setInterval" {
                        // Check if the return value is stored (parent is variable_declarator
                        // or assignment_expression).
                        let is_stored = Self::is_return_value_stored(call);
                        if !is_stored {
                            set_interval_nodes.push(call);
                        }
                    }

                    if fn_name == "clearInterval" {
                        has_clear_interval = true;
                    }
                }
            }
        }

        // Emit event listener leak findings
        if !has_remove_listener {
            for node in add_listener_nodes {
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "event_listener_leak".to_string(),
                    message:
                        "`addEventListener` without corresponding `removeEventListener` — potential memory leak"
                            .to_string(),
                    snippet: extract_snippet(source, node, 1),
                });
            }
        }

        // Emit uncleared interval findings
        if !has_clear_interval {
            for node in set_interval_nodes {
                let start = node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "uncleared_interval".to_string(),
                    message:
                        "`setInterval` return value not stored — interval cannot be cleared, potential memory leak"
                            .to_string(),
                    snippet: extract_snippet(source, node, 1),
                });
            }
        }

        // Emit unbounded array growth findings
        for node in growth_in_loop_nodes {
            let start = node.start_position();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                severity: "warning".to_string(),
                pipeline: self.name().to_string(),
                pattern: "unbounded_growth_in_loop".to_string(),
                message:
                    "array `.push()`/`.unshift()` inside a loop — potential unbounded memory growth"
                        .to_string(),
                snippet: extract_snippet(source, node, 1),
            });
        }

        findings
    }
}

impl MemoryLeakIndicatorsPipeline {
    /// Check if the call expression's return value is stored in a variable or assigned.
    fn is_return_value_stored(call_node: tree_sitter::Node) -> bool {
        if let Some(parent) = call_node.parent() {
            match parent.kind() {
                // const id = setInterval(...)
                "variable_declarator" => return true,
                // id = setInterval(...)
                "assignment_expression" => return true,
                _ => {}
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_add_event_listener_without_remove() {
        let src = "element.addEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "event_listener_leak");
    }

    #[test]
    fn skips_when_remove_event_listener_present() {
        let src = "\
element.addEventListener('click', handler);
element.removeEventListener('click', handler);";
        let findings = parse_and_check(src);
        let listener_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "event_listener_leak")
            .collect();
        assert!(listener_findings.is_empty());
    }

    #[test]
    fn detects_uncleared_set_interval() {
        let src = "setInterval(() => { doWork(); }, 1000);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "uncleared_interval");
    }

    #[test]
    fn skips_stored_set_interval() {
        let src = "const id = setInterval(() => { doWork(); }, 1000);";
        let findings = parse_and_check(src);
        let interval_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "uncleared_interval")
            .collect();
        assert!(interval_findings.is_empty());
    }

    #[test]
    fn skips_set_interval_with_clear_interval() {
        let src = "\
setInterval(() => { doWork(); }, 1000);
clearInterval(someId);";
        let findings = parse_and_check(src);
        let interval_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "uncleared_interval")
            .collect();
        assert!(interval_findings.is_empty());
    }

    #[test]
    fn detects_push_in_for_loop() {
        let src = "\
const results = [];
for (let i = 0; i < data.length; i++) {
    results.push(transform(data[i]));
}";
        let findings = parse_and_check(src);
        let growth_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_growth_in_loop")
            .collect();
        assert_eq!(growth_findings.len(), 1);
    }

    #[test]
    fn detects_unshift_in_while_loop() {
        let src = "\
while (queue.length > 0) {
    const item = queue.shift();
    results.unshift(process(item));
}";
        let findings = parse_and_check(src);
        let growth_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_growth_in_loop")
            .collect();
        assert_eq!(growth_findings.len(), 1);
    }

    #[test]
    fn ignores_push_outside_loop() {
        let src = "\
const items = [];
items.push(1);
items.push(2);";
        let findings = parse_and_check(src);
        let growth_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_growth_in_loop")
            .collect();
        assert!(growth_findings.is_empty());
    }

    #[test]
    fn no_findings_in_clean_code() {
        let src = "\
const id = setInterval(() => { tick(); }, 1000);
clearInterval(id);
element.addEventListener('click', handler);
element.removeEventListener('click', handler);";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
