use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;
use super::primitives::{extract_snippet, find_capture_index, node_text};

const GROWTH_METHODS: &[&str] = &["push", "insert", "extend"];

fn rust_lang() -> tree_sitter::Language {
    Language::Rust.tree_sitter_language()
}

pub struct MemoryLeakIndicatorsPipeline {
    scoped_call_query: Arc<Query>,
    loop_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        let scoped_call_query_str = r#"
(call_expression
  function: (scoped_identifier) @scoped_fn) @call
"#;
        let scoped_call_query = Query::new(&rust_lang(), scoped_call_query_str)
            .with_context(|| "failed to compile scoped call query for memory_leak_indicators")?;

        let loop_query_str = r#"
[
  (for_expression body: (block) @loop_body) @loop_expr
  (while_expression body: (block) @loop_body) @loop_expr
  (loop_expression body: (block) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&rust_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for memory_leak_indicators")?;

        let method_call_query_str = r#"
(call_expression
  function: (field_expression
    field: (field_identifier) @method_name)) @call
"#;
        let method_call_query = Query::new(&rust_lang(), method_call_query_str)
            .with_context(|| "failed to compile method call query for memory_leak_indicators")?;

        Ok(Self {
            scoped_call_query: Arc::new(scoped_call_query),
            loop_query: Arc::new(loop_query),
            method_call_query: Arc::new(method_call_query),
        })
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects potential memory leaks: Box::leak, mem::forget, ManuallyDrop::new, unbounded collection growth in loops"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Check for Box::leak, mem::forget, ManuallyDrop::new via scoped identifier calls
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.scoped_call_query, tree.root_node(), source);

            let fn_idx = find_capture_index(&self.scoped_call_query, "scoped_fn");
            let call_idx = find_capture_index(&self.scoped_call_query, "call");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_cap), Some(call_cap)) = (fn_node, call_node) {
                    let fn_text = node_text(fn_cap, source);

                    let (pattern, message) = if fn_text.ends_with("Box::leak")
                        || fn_text == "Box::leak"
                    {
                        (
                            "box_leak",
                            "Box::leak intentionally leaks memory — ensure this is necessary and the leaked reference has a bounded lifetime",
                        )
                    } else if fn_text.ends_with("mem::forget")
                        || fn_text == "mem::forget"
                        || fn_text == "std::mem::forget"
                    {
                        (
                            "mem_forget",
                            "mem::forget prevents Drop from running — may cause resource leaks (file handles, sockets, memory)",
                        )
                    } else if fn_text.ends_with("ManuallyDrop::new")
                        || fn_text == "ManuallyDrop::new"
                        || fn_text == "std::mem::ManuallyDrop::new"
                    {
                        (
                            "manually_drop",
                            "ManuallyDrop::new disables automatic drop — ensure manual cleanup is performed",
                        )
                    } else {
                        continue;
                    };

                    let start = call_cap.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: pattern.to_string(),
                        message: message.to_string(),
                        snippet: extract_snippet(source, call_cap, 1),
                    });
                }
            }
        }

        // Check for unbounded collection growth in loops (push, insert, extend)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.loop_query, tree.root_node(), source);

            let body_idx = find_capture_index(&self.loop_query, "loop_body");
            let loop_idx = find_capture_index(&self.loop_query, "loop_expr");

            while let Some(m) = matches.next() {
                let body_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == body_idx)
                    .map(|c| c.node);
                let loop_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == loop_idx)
                    .map(|c| c.node);

                if let (Some(body), Some(loop_n)) = (body_node, loop_node) {
                    let mut inner_cursor = QueryCursor::new();
                    inner_cursor.set_byte_range(body.byte_range());
                    let mut inner_matches =
                        inner_cursor.matches(&self.method_call_query, tree.root_node(), source);

                    let name_idx = find_capture_index(&self.method_call_query, "method_name");
                    let call_idx = find_capture_index(&self.method_call_query, "call");

                    while let Some(im) = inner_matches.next() {
                        let name_node = im
                            .captures
                            .iter()
                            .find(|c| c.index as usize == name_idx)
                            .map(|c| c.node);
                        let call_node = im
                            .captures
                            .iter()
                            .find(|c| c.index as usize == call_idx)
                            .map(|c| c.node);

                        if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                            let method_name = node_text(name_n, source);

                            if GROWTH_METHODS.contains(&method_name) {
                                let start = call_n.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "unbounded_growth_in_loop".to_string(),
                                    message: format!(
                                        "`.{method_name}()` inside loop without clear bound — collection may grow indefinitely causing memory exhaustion"
                                    ),
                                    snippet: extract_snippet(source, loop_n, 5),
                                });
                            }
                        }
                    }
                }
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
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.rs")
    }

    #[test]
    fn detects_box_leak() {
        let src = r#"
fn leak_string() {
    let s = String::from("hello");
    let leaked: &'static str = Box::leak(s.into_boxed_str());
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "box_leak");
    }

    #[test]
    fn detects_mem_forget() {
        let src = r#"
fn forget_value() {
    let v = vec![1, 2, 3];
    std::mem::forget(v);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "mem_forget");
    }

    #[test]
    fn detects_manually_drop_new() {
        let src = r#"
fn wrap_value() {
    let val = String::from("test");
    let md = ManuallyDrop::new(val);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "manually_drop");
    }

    #[test]
    fn detects_unbounded_push_in_loop() {
        let src = r#"
fn collect_events(rx: Receiver<Event>) {
    let mut events = Vec::new();
    loop {
        let event = rx.recv().unwrap();
        events.push(event);
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "unbounded_growth_in_loop"));
    }

    #[test]
    fn detects_insert_in_for_loop() {
        let src = r#"
fn index_items(items: &[Item]) {
    let mut map = HashMap::new();
    for item in items {
        map.insert(item.id, item.clone());
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "unbounded_growth_in_loop"
            && f.message.contains("insert")));
    }

    #[test]
    fn ignores_safe_code() {
        let src = r#"
fn safe_fn() {
    let v = vec![1, 2, 3];
    let sum: i32 = v.iter().sum();
    println!("{}", sum);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_push_outside_loop() {
        let src = r#"
fn build_list() {
    let mut v = Vec::new();
    v.push(1);
    v.push(2);
    v.push(3);
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
