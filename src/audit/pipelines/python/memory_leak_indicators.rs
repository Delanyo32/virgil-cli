use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{compile_call_query, extract_snippet, find_capture_index, node_text};

const LOOP_KINDS: &[&str] = &["for_statement", "while_statement"];

/// Methods that grow a collection without bound.
const GROWTH_METHODS: &[&str] = &["append", "extend", "insert", "add"];

pub struct MemoryLeakIndicatorsPipeline {
    call_query: Arc<Query>,
    class_method_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        let lang = Language::Python.tree_sitter_language();
        let class_method_query_str = r#"
(class_definition
  body: (block
    (function_definition
      name: (identifier) @method_name) @method_def))
"#;
        let class_method_query = Query::new(&lang, class_method_query_str)
            .with_context(|| "failed to compile class method query for Python")?;

        Ok(Self {
            call_query: compile_call_query()?,
            class_method_query: Arc::new(class_method_query),
        })
    }

    /// Check if a node is inside a `with` statement.
    fn is_inside_with_statement(node: tree_sitter::Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "with_statement" {
                return true;
            }
            current = parent.parent();
        }
        false
    }

    /// Check if a node is inside a loop.
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
        "Detects potential memory leaks: open() without with, unbounded growth in loops, __del__ methods"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // 1. Detect open() calls not inside `with` statements
        // 2. Detect .append()/.extend() inside loops
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

            let fn_expr_idx = find_capture_index(&self.call_query, "fn_expr");
            let call_idx = find_capture_index(&self.call_query, "call");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_expr_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_node), Some(call_node)) = (fn_node, call_node) {
                    // Check for open() not inside `with`
                    if fn_node.kind() == "identifier" && node_text(fn_node, source) == "open"
                        && !Self::is_inside_with_statement(call_node) {
                            let start = call_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "file_handle_leak".to_string(),
                                message: "`open()` called without a `with` statement — file handle may not be closed".to_string(),
                                snippet: extract_snippet(source, call_node, 1),
                            });
                        }

                    // Check for .append()/.extend()/.insert()/.add() inside loops
                    if fn_node.kind() == "attribute"
                        && let Some(attr) = fn_node.child_by_field_name("attribute") {
                            let method_name = node_text(attr, source);
                            if GROWTH_METHODS.contains(&method_name)
                                && Self::is_inside_loop(call_node)
                            {
                                let start = call_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "unbounded_growth".to_string(),
                                    message: format!(
                                        "`.{method_name}()` inside a loop — collection may grow without bound"
                                    ),
                                    snippet: extract_snippet(source, call_node, 1),
                                });
                            }
                        }
                }
            }
        }

        // 3. Detect __del__ method definitions
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.class_method_query, tree.root_node(), source);

            let method_name_idx = find_capture_index(&self.class_method_query, "method_name");
            let method_def_idx = find_capture_index(&self.class_method_query, "method_def");

            while let Some(m) = matches.next() {
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_name_idx)
                    .map(|c| c.node);
                let def_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_def_idx)
                    .map(|c| c.node);

                if let (Some(name_node), Some(def_node)) = (name_node, def_node)
                    && node_text(name_node, source) == "__del__" {
                        let start = def_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "manual_resource_management".to_string(),
                            message: "`__del__` method defined — often indicates manual resource management; prefer context managers".to_string(),
                            snippet: extract_snippet(source, def_node, 2),
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
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.py")
    }

    #[test]
    fn detects_open_without_with() {
        let src = "\
f = open('data.txt')
data = f.read()
f.close()
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "file_handle_leak");
    }

    #[test]
    fn ignores_open_inside_with() {
        let src = "\
with open('data.txt') as f:
    data = f.read()
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_append_in_loop() {
        let src = "\
results = []
for item in items:
    results.append(process(item))
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_growth");
    }

    #[test]
    fn ignores_append_outside_loop() {
        let src = "\
results = []
results.append(42)
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_del_method() {
        let src = "\
class Resource:
    def __del__(self):
        self.cleanup()
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "manual_resource_management");
    }

    #[test]
    fn ignores_normal_methods() {
        let src = "\
class Resource:
    def __init__(self):
        pass
    def close(self):
        pass
";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_extend_in_while_loop() {
        let src = "\
data = []
while True:
    chunk = read_chunk()
    data.extend(chunk)
";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unbounded_growth");
    }
}
