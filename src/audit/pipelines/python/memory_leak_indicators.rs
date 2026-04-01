use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
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

    /// Check if a node is inside a `try` statement that has a `finally_clause`.
    fn is_inside_try_with_finally(node: Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "try_statement" {
                // Check if this try statement has a finally_clause child
                let mut child_cursor = parent.walk();
                for child in parent.children(&mut child_cursor) {
                    if child.kind() == "finally_clause" {
                        return true;
                    }
                }
            }
            current = parent.parent();
        }
        false
    }

    /// Find a `call` node at a given 0-based row in the tree by walking it.
    fn find_call_node_at_line(node: Node, target_row: usize) -> Option<Node> {
        if node.kind() == "call" && node.start_position().row == target_row {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = Self::find_call_node_at_line(child, target_row) {
                return Some(found);
            }
        }
        None
    }

    /// Walk up from a node to find the enclosing `function_definition`.
    fn find_enclosing_function(node: Node) -> Option<Node> {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_definition" {
                return Some(parent);
            }
            current = parent.parent();
        }
        None
    }

    /// Walk up from a node to find the enclosing loop (`for_statement` or `while_statement`).
    fn find_enclosing_loop(node: Node) -> Option<Node> {
        let mut current = node.parent();
        while let Some(parent) = current {
            if LOOP_KINDS.contains(&parent.kind()) {
                return Some(parent);
            }
            current = parent.parent();
        }
        None
    }

    /// Extract the collection variable name from a `.append()`/`.extend()` call.
    /// For `results.append(x)`, the function node is `results.append` (an `attribute` node),
    /// and the object (left part) is `results`.
    fn extract_collection_name<'a>(call_node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
        let fn_node = call_node.child_by_field_name("function")?;
        if fn_node.kind() != "attribute" {
            return None;
        }
        let object = fn_node.child_by_field_name("object")?;
        if object.kind() == "identifier" {
            return Some(node_text(object, source));
        }
        None
    }

    /// Check if a variable is initialized as an empty list in the function body.
    /// Looks for patterns like `name = []` or `name = list()`.
    fn is_initialized_as_empty_list(func_node: Node, source: &[u8], name: &str) -> bool {
        Self::walk_for_assignment(func_node, source, name)
    }

    fn walk_for_assignment(node: Node, source: &[u8], name: &str) -> bool {
        if node.kind() == "assignment"
            && let Some(left) = node.child_by_field_name("left")
            && left.kind() == "identifier"
            && node_text(left, source) == name
            && let Some(right) = node.child_by_field_name("right")
        {
            // Check for `[]`
            if right.kind() == "list" && right.named_child_count() == 0 {
                return true;
            }
            // Check for `list()`
            if right.kind() == "call"
                && let Some(fn_node) = right.child_by_field_name("function")
                && fn_node.kind() == "identifier"
                && node_text(fn_node, source) == "list"
            {
                return true;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::walk_for_assignment(child, source, name) {
                return true;
            }
        }
        false
    }

    /// Check if a variable name appears in a `return` statement in the function body.
    fn is_returned_in_function(func_node: Node, source: &[u8], name: &str) -> bool {
        Self::walk_for_return(func_node, source, name)
    }

    fn walk_for_return(node: Node, source: &[u8], name: &str) -> bool {
        if node.kind() == "return_statement" {
            // Check if any child of the return statement references the name
            return Self::contains_identifier(node, source, name);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::walk_for_return(child, source, name) {
                return true;
            }
        }
        false
    }

    /// Check if a node (or its descendants) contains an identifier with the given name.
    fn contains_identifier(node: Node, source: &[u8], name: &str) -> bool {
        if node.kind() == "identifier" && node_text(node, source) == name {
            return true;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::contains_identifier(child, source, name) {
                return true;
            }
        }
        false
    }

    /// Extract parameter names from a function_definition node.
    fn extract_parameter_names<'a>(func_node: Node<'a>, source: &'a [u8]) -> Vec<&'a str> {
        let mut names = Vec::new();
        if let Some(params) = func_node.child_by_field_name("parameters") {
            let mut cursor = params.walk();
            for child in params.children(&mut cursor) {
                match child.kind() {
                    "identifier" => {
                        names.push(node_text(child, source));
                    }
                    "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                        // The name is the first identifier child or `name` field
                        if let Some(name_node) = child.child_by_field_name("name") {
                            names.push(node_text(name_node, source));
                        } else {
                            // Fallback: first identifier child
                            let mut inner_cursor = child.walk();
                            for inner_child in child.children(&mut inner_cursor) {
                                if inner_child.kind() == "identifier" {
                                    names.push(node_text(inner_child, source));
                                    break;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        names
    }

    /// Check if the iterator of a for_statement is a function parameter.
    fn is_for_iterator_a_parameter(loop_node: Node, func_node: Node, source: &[u8]) -> bool {
        // for_statement structure: `for <left> in <right>:`
        // The `right` field is the iterator expression
        if loop_node.kind() != "for_statement" {
            return false;
        }
        if let Some(right) = loop_node.child_by_field_name("right")
            && right.kind() == "identifier"
        {
            let iter_name = node_text(right, source);
            let params = Self::extract_parameter_names(func_node, source);
            return params.contains(&iter_name);
        }
        false
    }

    /// Determine if an unbounded_growth finding is a result builder pattern.
    fn is_result_builder(call_node: Node, source: &[u8]) -> bool {
        let collection_name = match Self::extract_collection_name(call_node, source) {
            Some(name) => name,
            None => return false,
        };

        let func_node = match Self::find_enclosing_function(call_node) {
            Some(f) => f,
            None => return false,
        };

        let initialized = Self::is_initialized_as_empty_list(func_node, source, collection_name);
        let returned = Self::is_returned_in_function(func_node, source, collection_name);

        initialized && returned
    }

    /// Determine if the loop iterating over bounded (parameter-driven) input.
    fn is_bounded_loop_iteration(call_node: Node, source: &[u8]) -> bool {
        let loop_node = match Self::find_enclosing_loop(call_node) {
            Some(l) => l,
            None => return false,
        };

        let func_node = match Self::find_enclosing_function(call_node) {
            Some(f) => f,
            None => return false,
        };

        Self::is_for_iterator_a_parameter(loop_node, func_node, source)
    }
}

impl MemoryLeakIndicatorsPipeline {
    fn check_tree_sitter(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
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
                    if fn_node.kind() == "identifier"
                        && node_text(fn_node, source) == "open"
                        && !Self::is_inside_with_statement(call_node)
                    {
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
                        && let Some(attr) = fn_node.child_by_field_name("attribute")
                    {
                        let method_name = node_text(attr, source);
                        if GROWTH_METHODS.contains(&method_name) && Self::is_inside_loop(call_node)
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
                    && node_text(name_node, source) == "__del__"
                {
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

impl GraphPipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects potential memory leaks: open() without with, unbounded growth in loops, __del__ methods"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let findings = self.check_tree_sitter(ctx.tree, ctx.source, ctx.file_path);
        let root = ctx.tree.root_node();

        findings
            .into_iter()
            .filter(|finding| {
                match finding.pattern.as_str() {
                    "unbounded_growth" => {
                        // finding.line is 1-based, tree-sitter rows are 0-based
                        let target_row = (finding.line - 1) as usize;

                        if let Some(call_node) = Self::find_call_node_at_line(root, target_row) {
                            // Suppress if this is a result builder pattern
                            if Self::is_result_builder(call_node, ctx.source) {
                                return false;
                            }

                            // Suppress if the loop iterates over a function parameter
                            if Self::is_bounded_loop_iteration(call_node, ctx.source) {
                                return false;
                            }
                        }

                        true
                    }
                    "file_handle_leak" => {
                        let target_row = (finding.line - 1) as usize;

                        if let Some(call_node) = Self::find_call_node_at_line(root, target_row) {
                            // Suppress if inside a try block with a finally clause
                            if Self::is_inside_try_with_finally(call_node) {
                                return false;
                            }
                        }

                        true
                    }
                    // Pass through all other patterns unchanged
                    _ => true,
                }
            })
            .collect()
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
        pipeline.check_tree_sitter(&tree, source.as_bytes(), "test.py")
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

    // --- check_with_context tests ---

    fn parse_and_check_with_context(source: &str) -> Vec<AuditFinding> {
        use std::collections::HashMap;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Python.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        let id_counts = HashMap::new();
        let graph = crate::graph::CodeGraph::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.py",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn context_suppresses_result_builder_pattern() {
        let src = "\
def get_items(data):
    results = []
    for item in data:
        results.append(process(item))
    return results
";
        let findings = parse_and_check_with_context(src);
        // Should be suppressed: results is initialized as [] and returned
        let unbounded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_growth")
            .collect();
        assert!(
            unbounded.is_empty(),
            "result builder pattern should be suppressed, but got: {:?}",
            unbounded
        );
    }

    #[test]
    fn context_flags_genuinely_unbounded_growth() {
        let src = "\
class Tracker:
    def __init__(self):
        self.history = []
    def track(self, event):
        while True:
            self.history.append(event)
";
        let findings = parse_and_check_with_context(src);
        let unbounded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_growth")
            .collect();
        assert_eq!(
            unbounded.len(),
            1,
            "genuinely unbounded growth should still be flagged"
        );
    }

    #[test]
    fn context_suppresses_open_inside_try_finally() {
        let src = "\
f = None
try:
    f = open('data.txt')
    data = f.read()
finally:
    if f:
        f.close()
";
        let findings = parse_and_check_with_context(src);
        let file_leaks: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "file_handle_leak")
            .collect();
        assert!(
            file_leaks.is_empty(),
            "open() inside try/finally should be suppressed, but got: {:?}",
            file_leaks
        );
    }

    #[test]
    fn context_suppresses_bounded_loop_with_param_iterator() {
        let src = "\
def transform(items):
    output = []
    for x in items:
        output.append(x * 2)
    return output
";
        let findings = parse_and_check_with_context(src);
        let unbounded: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unbounded_growth")
            .collect();
        assert!(
            unbounded.is_empty(),
            "bounded loop iterating over parameter should be suppressed"
        );
    }

    #[test]
    fn context_still_flags_open_without_try_finally() {
        let src = "\
f = open('data.txt')
data = f.read()
f.close()
";
        let findings = parse_and_check_with_context(src);
        let file_leaks: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "file_handle_leak")
            .collect();
        assert_eq!(
            file_leaks.len(),
            1,
            "open() without try/finally or with should still be flagged"
        );
    }

    #[test]
    fn context_passes_through_manual_resource_management() {
        let src = "\
class Resource:
    def __del__(self):
        self.cleanup()
";
        let findings = parse_and_check_with_context(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "manual_resource_management");
    }
}
