use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::NodePipeline;
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{compile_function_query, extract_snippet, find_capture_index, node_text};

/// Methods that mutate arrays/objects/sets/maps in place.
const MUTATING_METHODS: &[&str] = &[
    "push",
    "pop",
    "shift",
    "unshift",
    "splice",
    "sort",
    "reverse",
    "fill",
    "copyWithin",
    "delete",
    "clear",
    "add",
    "set",
];

pub struct ArgumentMutationPipeline {
    func_query: Arc<Query>,
}

impl ArgumentMutationPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            func_query: compile_function_query()?,
        })
    }

    /// Extract parameter names from formal_parameters node.
    fn extract_param_names(params_node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = params_node.walk();
        for child in params_node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    names.push(node_text(child, source).to_string());
                }
                "assignment_pattern" => {
                    if let Some(left) = child.child_by_field_name("left") {
                        if left.kind() == "identifier" {
                            names.push(node_text(left, source).to_string());
                        }
                    }
                }
                "rest_pattern" => {
                    let mut inner = child.walk();
                    for gc in child.named_children(&mut inner) {
                        if gc.kind() == "identifier" {
                            names.push(node_text(gc, source).to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        names
    }

    fn find_mutations(
        body_node: tree_sitter::Node,
        param_names: &[String],
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        Self::walk_for_mutations(body_node, param_names, source, file_path, pipeline_name, findings);
    }

    fn walk_for_mutations(
        node: tree_sitter::Node,
        param_names: &[String],
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        // Don't recurse into nested functions — they have their own params
        if node.kind() == "function_declaration"
            || node.kind() == "function_expression"
            || node.kind() == "arrow_function"
        {
            return;
        }

        let mut found = false;

        match node.kind() {
            // 1. assignment_expression: param.x = val, param[i] = val
            "assignment_expression" => {
                if let Some(lhs) = node.child_by_field_name("left") {
                    if Self::is_param_access(lhs, param_names, source) {
                        Self::emit_finding(
                            node, param_names, lhs, source, file_path, pipeline_name, findings,
                        );
                        found = true;
                    }
                }
            }
            // 2. augmented_assignment_expression: param.x += val
            "augmented_assignment_expression" => {
                if let Some(lhs) = node.child_by_field_name("left") {
                    if Self::is_param_access(lhs, param_names, source) {
                        Self::emit_finding(
                            node, param_names, lhs, source, file_path, pipeline_name, findings,
                        );
                        found = true;
                    }
                }
            }
            // 3. update_expression: param.x++ or ++param.x
            "update_expression" => {
                if let Some(arg) = node.child_by_field_name("argument") {
                    if Self::is_param_access(arg, param_names, source) {
                        Self::emit_finding(
                            node, param_names, arg, source, file_path, pipeline_name, findings,
                        );
                        found = true;
                    }
                }
            }
            // 4. unary_expression: delete param.x
            "unary_expression" => {
                if let Some(op) = node.child_by_field_name("operator") {
                    if node_text(op, source) == "delete" {
                        if let Some(arg) = node.child_by_field_name("argument") {
                            if Self::is_param_access(arg, param_names, source) {
                                Self::emit_finding(
                                    node, param_names, arg, source, file_path, pipeline_name,
                                    findings,
                                );
                                found = true;
                            }
                        }
                    }
                }
            }
            // 5. call_expression: param.push(...), Object.assign(param, ...)
            "call_expression" => {
                if Self::is_mutating_call(node, param_names, source) {
                    let lhs = node
                        .child_by_field_name("function")
                        .unwrap_or(node);
                    Self::emit_finding(
                        node, param_names, lhs, source, file_path, pipeline_name, findings,
                    );
                    found = true;
                }
            }
            _ => {}
        }

        if !found {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                Self::walk_for_mutations(
                    child,
                    param_names,
                    source,
                    file_path,
                    pipeline_name,
                    findings,
                );
            }
        }
    }

    /// Check if a node is a member_expression or subscript_expression rooted in a parameter.
    fn is_param_access(
        node: tree_sitter::Node,
        param_names: &[String],
        source: &[u8],
    ) -> bool {
        match node.kind() {
            "member_expression" | "subscript_expression" => {
                let root = Self::root_object(node);
                root.kind() == "identifier" && param_names.iter().any(|p| p == node_text(root, source))
            }
            _ => false,
        }
    }

    /// Check if this call expression is a mutating call on a parameter.
    fn is_mutating_call(
        call_node: tree_sitter::Node,
        param_names: &[String],
        source: &[u8],
    ) -> bool {
        if let Some(func) = call_node.child_by_field_name("function") {
            // param.push(...) -- method call on param
            if func.kind() == "member_expression" {
                if let Some(prop) = func.child_by_field_name("property") {
                    let method_name = node_text(prop, source);
                    if MUTATING_METHODS.contains(&method_name) {
                        let root = Self::root_object(func);
                        if root.kind() == "identifier"
                            && param_names.iter().any(|p| p == node_text(root, source))
                        {
                            return true;
                        }
                    }
                }
            }

            // Object.assign(param, ...) or Object.defineProperty(param, ...)
            if func.kind() == "member_expression" {
                if let Some(obj) = func.child_by_field_name("object") {
                    if let Some(prop) = func.child_by_field_name("property") {
                        let obj_name = node_text(obj, source);
                        let method = node_text(prop, source);
                        if obj_name == "Object"
                            && (method == "assign" || method == "defineProperty")
                        {
                            // First argument should be a parameter
                            if let Some(args) = call_node.child_by_field_name("arguments") {
                                if let Some(first_arg) = args.named_child(0) {
                                    if first_arg.kind() == "identifier"
                                        && param_names
                                            .iter()
                                            .any(|p| p == node_text(first_arg, source))
                                    {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    fn emit_finding(
        node: tree_sitter::Node,
        param_names: &[String],
        access_node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        pipeline_name: &str,
        findings: &mut Vec<AuditFinding>,
    ) {
        // Determine which param is being mutated
        let root = Self::root_object(access_node);
        let param_name = if root.kind() == "identifier" {
            let text = node_text(root, source);
            if param_names.iter().any(|p| p == text) {
                text
            } else {
                "parameter"
            }
        } else {
            "parameter"
        };

        if is_nolint_suppressed(source, node, pipeline_name) {
            return;
        }

        let start = node.start_position();
        findings.push(AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "warning".to_string(),
            pipeline: pipeline_name.to_string(),
            pattern: "argument_mutation".to_string(),
            message: format!(
                "mutating parameter `{param_name}` — creates hidden side effects for callers"
            ),
            snippet: extract_snippet(source, node, 1),
        });
    }

    fn root_object(node: tree_sitter::Node) -> tree_sitter::Node {
        let mut current = node;
        loop {
            if let Some(obj) = current.child_by_field_name("object") {
                if obj.kind() == "member_expression" || obj.kind() == "subscript_expression" {
                    current = obj;
                } else {
                    return obj;
                }
            } else {
                return current;
            }
        }
    }
}

impl NodePipeline for ArgumentMutationPipeline {
    fn name(&self) -> &str {
        "argument_mutation"
    }

    fn description(&self) -> &str {
        "Detects mutation of function parameters — creates hidden side effects"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.func_query, tree.root_node(), source);

        let params_idx = find_capture_index(&self.func_query, "params");
        let body_idx = find_capture_index(&self.func_query, "body");

        while let Some(m) = matches.next() {
            let params_cap = m.captures.iter().find(|c| c.index as usize == params_idx);
            let body_cap = m.captures.iter().find(|c| c.index as usize == body_idx);

            if let (Some(params), Some(body)) = (params_cap, body_cap) {
                let param_names = Self::extract_param_names(params.node, source);
                if param_names.is_empty() {
                    continue;
                }

                Self::find_mutations(
                    body.node,
                    &param_names,
                    source,
                    file_path,
                    self.name(),
                    &mut findings,
                );
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
        let pipeline = ArgumentMutationPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_argument_mutation() {
        let src = "function foo(obj) { obj.name = 'bar'; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "argument_mutation");
        assert!(findings[0].message.contains("obj"));
    }

    #[test]
    fn skips_local_variable_mutation() {
        let src = "function foo(obj) { let local = {}; local.name = 'bar'; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_no_params() {
        let src = "function foo() { x.name = 'bar'; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_deep_mutation() {
        let src = "function foo(config) { config.nested.deep = true; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("config"));
    }

    #[test]
    fn detects_arrow_function_mutation() {
        let src = "const foo = (obj) => { obj.x = 1; };";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    // --- New tests for expanded detection ---

    #[test]
    fn detects_subscript_mutation() {
        let src = "function f(arr) { arr[0] = 'x'; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("arr"));
    }

    #[test]
    fn detects_push_mutation() {
        let src = "function f(arr) { arr.push(1); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("arr"));
    }

    #[test]
    fn detects_sort_mutation() {
        let src = "function f(arr) { arr.sort(); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_splice_mutation() {
        let src = "function f(arr) { arr.splice(0, 1); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_delete_mutation() {
        let src = "function f(obj) { delete obj.key; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_augmented_assignment() {
        let src = "function f(obj) { obj.count += 1; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_object_assign() {
        let src = "function f(obj) { Object.assign(obj, defaults); }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_non_mutating_method() {
        let src = "function f(arr) { arr.map(x => x + 1); }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "function f(obj) {\n// NOLINT(argument_mutation)\nobj.x = 1;\n}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
