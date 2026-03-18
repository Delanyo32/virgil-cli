use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const LOOP_KINDS: &[&str] = &["for_statement", "while_statement", "foreach_statement"];

fn php_lang() -> tree_sitter::Language {
    Language::Php.tree_sitter_language()
}

pub struct MemoryLeakIndicatorsPipeline {
    fn_call_query: Arc<Query>,
    loop_query: Arc<Query>,
    member_assignment_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        let fn_call_query_str = r#"
(function_call_expression
  function: (name) @fn_name
  arguments: (arguments) @args) @call
"#;
        let fn_call_query = Query::new(&php_lang(), fn_call_query_str)
            .with_context(|| "failed to compile function_call query for PHP memory_leak")?;

        let loop_query_str = r#"
[
  (for_statement body: (compound_statement) @loop_body) @loop_expr
  (while_statement body: (compound_statement) @loop_body) @loop_expr
  (foreach_statement body: (compound_statement) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&php_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for PHP memory_leak")?;

        // Matches $this->prop = $this (circular reference pattern)
        let member_assignment_str = r#"
(assignment_expression
  left: (member_access_expression
    object: (variable_name) @obj) @member
  right: (_) @rhs) @assign
"#;
        let member_assignment_query = Query::new(&php_lang(), member_assignment_str)
            .with_context(|| "failed to compile member_assignment query for PHP memory_leak")?;

        Ok(Self {
            fn_call_query: Arc::new(fn_call_query),
            loop_query: Arc::new(loop_query),
            member_assignment_query: Arc::new(member_assignment_query),
        })
    }

    /// Detect fopen() calls without a corresponding fclose() in the same function body.
    fn check_unclosed_resources(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_call_query, "fn_name");
        let call_idx = find_capture_index(&self.fn_call_query, "call");

        // Collect all fopen and fclose call nodes
        let mut fopen_calls: Vec<tree_sitter::Node> = Vec::new();
        let mut has_fclose = false;

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                let fn_name = node_text(name_n, source);
                match fn_name {
                    "fopen" => {
                        fopen_calls.push(call_n);
                    }
                    "fclose" => {
                        has_fclose = true;
                    }
                    _ => {}
                }
            }
        }

        // If there are fopen calls but no fclose anywhere, flag them
        if !has_fclose {
            for call_n in fopen_calls {
                let start = call_n.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "unclosed_resource".to_string(),
                    message: "`fopen()` without corresponding `fclose()` — resource may leak"
                        .to_string(),
                    snippet: extract_snippet(source, call_n, 2),
                });
            }
        }
    }

    /// Detect array growth patterns inside loops: array_push() or $arr[] = ...
    fn check_array_growth_in_loops(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
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
                // Check for array_push() inside the loop body
                self.check_array_push_in_body(tree, source, body, loop_n, file_path, findings);

                // Check for $arr[] = ... pattern inside the loop body
                self.check_subscript_append_in_body(source, body, loop_n, file_path, findings);
            }
        }
    }

    fn check_array_push_in_body(
        &self,
        tree: &Tree,
        source: &[u8],
        body: tree_sitter::Node,
        loop_node: tree_sitter::Node,
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(body.byte_range());
        let mut matches = cursor.matches(&self.fn_call_query, tree.root_node(), source);

        let fn_name_idx = find_capture_index(&self.fn_call_query, "fn_name");
        let call_idx = find_capture_index(&self.fn_call_query, "call");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_name_idx)
                .map(|c| c.node);
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == call_idx)
                .map(|c| c.node);

            if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                let fn_name = node_text(name_n, source);
                if fn_name == "array_push" {
                    let start = call_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "array_growth_in_loop".to_string(),
                        message: "`array_push()` inside a loop — array may grow unbounded, consider pre-allocating or limiting size"
                            .to_string(),
                        snippet: extract_snippet(source, loop_node, 5),
                    });
                }
            }
        }
    }

    /// Walk the loop body subtree looking for `$arr[] = ...` patterns.
    /// These appear as assignment_expression with a subscript_expression on the left
    /// where the subscript has no index (empty brackets).
    fn check_subscript_append_in_body(
        &self,
        source: &[u8],
        body: tree_sitter::Node,
        loop_node: tree_sitter::Node,
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut stack = vec![body];
        while let Some(current) = stack.pop() {
            if current.kind() == "assignment_expression" {
                if let Some(left) = current.child_by_field_name("left") {
                    if left.kind() == "subscript_expression" {
                        // Check if this is an append: $arr[] = ... (no index specified)
                        // In tree-sitter-php, $arr[] has subscript_expression with
                        // only the object child and no index child
                        let text = node_text(left, source);
                        if text.ends_with("[]") {
                            let start = current.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "array_growth_in_loop".to_string(),
                                message: "Array append (`$arr[] = ...`) inside a loop — array may grow unbounded"
                                    .to_string(),
                                snippet: extract_snippet(source, loop_node, 5),
                            });
                        }
                    }
                }
            }

            for i in 0..current.named_child_count() {
                if let Some(child) = current.named_child(i) {
                    // Don't descend into nested loops (they will be matched separately)
                    if !LOOP_KINDS.contains(&child.kind()) {
                        stack.push(child);
                    }
                }
            }
        }
    }

    /// Detect circular reference patterns: $this->prop = $this
    fn check_circular_references(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.member_assignment_query, tree.root_node(), source);

        let obj_idx = find_capture_index(&self.member_assignment_query, "obj");
        let rhs_idx = find_capture_index(&self.member_assignment_query, "rhs");
        let assign_idx = find_capture_index(&self.member_assignment_query, "assign");

        while let Some(m) = matches.next() {
            let obj_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == obj_idx)
                .map(|c| c.node);
            let rhs_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == rhs_idx)
                .map(|c| c.node);
            let assign_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == assign_idx)
                .map(|c| c.node);

            if let (Some(obj_n), Some(rhs_n), Some(assign_n)) = (obj_node, rhs_node, assign_node)
            {
                let obj_text = node_text(obj_n, source);
                let rhs_text = node_text(rhs_n, source);

                // Check if $this->something = $this
                if obj_text == "$this" && rhs_text == "$this" {
                    let start = assign_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "circular_reference".to_string(),
                        message:
                            "`$this->` stores reference to `$this` — potential circular reference causing memory leak"
                                .to_string(),
                        snippet: extract_snippet(source, assign_n, 2),
                    });
                }
            }
        }
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects patterns that may cause memory leaks: unclosed resources, unbounded array growth, circular references"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        self.check_unclosed_resources(tree, source, file_path, &mut findings);
        self.check_array_growth_in_loops(tree, source, file_path, &mut findings);
        self.check_circular_references(tree, source, file_path, &mut findings);
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
            .set_language(&Language::Php.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.php")
    }

    #[test]
    fn detects_fopen_without_fclose() {
        let src = "<?php\n$fp = fopen('file.txt', 'r');\n$data = fread($fp, 1024);\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unclosed_resource");
        assert!(findings[0].message.contains("fopen"));
    }

    #[test]
    fn ignores_fopen_with_fclose() {
        let src = "<?php\n$fp = fopen('file.txt', 'r');\n$data = fread($fp, 1024);\nfclose($fp);\n";
        let findings = parse_and_check(src);
        let resource_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unclosed_resource")
            .collect();
        assert!(resource_findings.is_empty());
    }

    #[test]
    fn detects_array_push_in_loop() {
        let src = "<?php\n$results = [];\nforeach ($items as $item) {\n    array_push($results, process($item));\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "array_growth_in_loop");
        assert!(findings[0].message.contains("array_push"));
    }

    #[test]
    fn detects_array_append_in_loop() {
        let src = "<?php\n$results = [];\nwhile ($row = fetch()) {\n    $results[] = $row;\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "array_growth_in_loop");
    }

    #[test]
    fn detects_circular_reference() {
        let src = "<?php\nclass Foo {\n    public function init() {\n        $this->parent = $this;\n    }\n}\n";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "circular_reference");
    }

    #[test]
    fn ignores_normal_property_assignment() {
        let src = "<?php\nclass Foo {\n    public function init() {\n        $this->name = 'hello';\n    }\n}\n";
        let findings = parse_and_check(src);
        let circular_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "circular_reference")
            .collect();
        assert!(circular_findings.is_empty());
    }

    #[test]
    fn ignores_array_push_outside_loop() {
        let src = "<?php\n$results = [];\narray_push($results, 'item');\n";
        let findings = parse_and_check(src);
        let growth_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "array_growth_in_loop")
            .collect();
        assert!(growth_findings.is_empty());
    }
}
