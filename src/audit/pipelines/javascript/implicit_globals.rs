use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{
    compile_assignment_expression_query, extract_snippet, find_capture_index, node_text,
};

const KNOWN_GLOBALS: &[&str] = &[
    // Node.js / runtime
    "window",
    "document",
    "module",
    "exports",
    "require",
    "process",
    "Buffer",
    "__dirname",
    "__filename",
    "global",
    "globalThis",
    "console",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "setImmediate",
    "clearImmediate",
    // Language fundamentals
    "undefined",
    "NaN",
    "Infinity",
    // Built-in constructors / objects
    "JSON",
    "Math",
    "Array",
    "Object",
    "String",
    "Number",
    "Boolean",
    "Symbol",
    "BigInt",
    "Promise",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "WeakRef",
    "Date",
    "RegExp",
    "Proxy",
    "Reflect",
    "Intl",
    // Error types
    "Error",
    "TypeError",
    "RangeError",
    "ReferenceError",
    "SyntaxError",
    "EvalError",
    "URIError",
    "AggregateError",
    // Global functions
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "eval",
    "encodeURIComponent",
    "decodeURIComponent",
    "encodeURI",
    "decodeURI",
    "atob",
    "btoa",
    "fetch",
    "queueMicrotask",
    "structuredClone",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "requestIdleCallback",
    "cancelIdleCallback",
    // Web APIs
    "URL",
    "URLSearchParams",
    "AbortController",
    "AbortSignal",
    "TextEncoder",
    "TextDecoder",
    "Headers",
    "Request",
    "Response",
    "FormData",
    "Blob",
    "File",
    "FileReader",
    "ReadableStream",
    "WritableStream",
    "TransformStream",
    "WebSocket",
    "EventSource",
    "XMLHttpRequest",
    "Worker",
    "SharedWorker",
    "MessageChannel",
    "MessagePort",
    "BroadcastChannel",
    "Notification",
    "IntersectionObserver",
    "MutationObserver",
    "ResizeObserver",
    "PerformanceObserver",
    "CustomEvent",
    "Event",
    "EventTarget",
    "HTMLElement",
    "SVGElement",
    // Browser globals
    "navigator",
    "location",
    "history",
    "performance",
    "crypto",
    "self",
    "alert",
    "confirm",
    "prompt",
    "screen",
    // Typed arrays
    "ArrayBuffer",
    "SharedArrayBuffer",
    "DataView",
    "Float32Array",
    "Float64Array",
    "Int8Array",
    "Int16Array",
    "Int32Array",
    "Uint8Array",
    "Uint16Array",
    "Uint32Array",
    "Uint8ClampedArray",
];

pub struct ImplicitGlobalsPipeline {
    assign_query: Arc<Query>,
}

impl ImplicitGlobalsPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            assign_query: compile_assignment_expression_query()?,
        })
    }

    /// Walk up from `node` through all enclosing scopes (function -> program)
    /// checking if there's a declaration with the same name at any level.
    /// This handles imports at program scope being visible inside nested functions.
    fn has_declaration_in_scope(node: tree_sitter::Node, name: &str, source: &[u8]) -> bool {
        let mut scope = node.parent();
        while let Some(s) = scope {
            match s.kind() {
                "function_declaration" | "function_expression" | "arrow_function" | "program" => {
                    if Self::scope_has_declaration(s, name, source) {
                        return true;
                    }
                    // Continue walking up to check outer scopes (e.g., imports at program level)
                    if s.kind() == "program" {
                        return false;
                    }
                    scope = s.parent();
                }
                _ => {
                    scope = s.parent();
                }
            }
        }
        false
    }

    fn scope_has_declaration(scope_node: tree_sitter::Node, name: &str, source: &[u8]) -> bool {
        Self::search_declarations(scope_node, name, source)
    }

    fn search_declarations(
        node: tree_sitter::Node,
        name: &str,
        source: &[u8],
    ) -> bool {
        match node.kind() {
            "variable_declarator" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    // Direct identifier binding
                    if name_node.kind() == "identifier" && node_text(name_node, source) == name {
                        return true;
                    }
                    // Destructured bindings: { a, b } = ... or [a, b] = ...
                    if name_node.kind() == "object_pattern" || name_node.kind() == "array_pattern" {
                        if Self::pattern_contains_name(name_node, name, source) {
                            return true;
                        }
                    }
                }
            }
            "formal_parameters" => {
                let mut child_cursor = node.walk();
                for child in node.named_children(&mut child_cursor) {
                    if child.kind() == "identifier" && node_text(child, source) == name {
                        return true;
                    }
                    // Destructured parameters: function({a, b}) or function([a, b])
                    if child.kind() == "object_pattern" || child.kind() == "array_pattern" {
                        if Self::pattern_contains_name(child, name, source) {
                            return true;
                        }
                    }
                    // Default parameter: function(x = 1) -- x is inside assignment_pattern
                    if child.kind() == "assignment_pattern" {
                        if let Some(left) = child.child_by_field_name("left") {
                            if left.kind() == "identifier" && node_text(left, source) == name {
                                return true;
                            }
                            if left.kind() == "object_pattern" || left.kind() == "array_pattern" {
                                if Self::pattern_contains_name(left, name, source) {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            "function_declaration" => {
                // function foo() {} declares `foo` in the enclosing scope
                if let Some(name_node) = node.child_by_field_name("name") {
                    if name_node.kind() == "identifier" && node_text(name_node, source) == name {
                        return true;
                    }
                }
            }
            "class_declaration" => {
                // class Foo {} declares `Foo` in the enclosing scope
                if let Some(name_node) = node.child_by_field_name("name") {
                    if name_node.kind() == "identifier" && node_text(name_node, source) == name {
                        return true;
                    }
                }
            }
            "import_statement" => {
                // import { x } from 'mod'; import x from 'mod'; import * as x from 'mod';
                if Self::import_declares_name(node, name, source) {
                    return true;
                }
            }
            "catch_clause" => {
                // catch(e) { ... } declares `e`
                if let Some(param) = node.child_by_field_name("parameter") {
                    if param.kind() == "identifier" && node_text(param, source) == name {
                        return true;
                    }
                    if param.kind() == "object_pattern" || param.kind() == "array_pattern" {
                        if Self::pattern_contains_name(param, name, source) {
                            return true;
                        }
                    }
                }
            }
            "for_in_statement" | "for_of_statement" => {
                // for (var/let/const x in/of ...) or for (x in/of ...)
                if let Some(left) = node.child_by_field_name("left") {
                    if left.kind() == "identifier" && node_text(left, source) == name {
                        return true;
                    }
                    // Also check inside lexical_declaration/variable_declaration
                    if Self::search_declarations(left, name, source) {
                        return true;
                    }
                }
            }
            _ => {}
        }

        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            // Don't recurse into nested functions — they have their own scope
            if child.kind() == "function_declaration"
                || child.kind() == "function_expression"
                || child.kind() == "arrow_function"
            {
                // But still check function_declaration's name (it's declared in this scope)
                if child.kind() == "function_declaration" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if name_node.kind() == "identifier"
                            && node_text(name_node, source) == name
                        {
                            return true;
                        }
                    }
                }
                continue;
            }
            if Self::search_declarations(child, name, source) {
                return true;
            }
        }

        false
    }

    /// Recursively extract identifiers from destructuring patterns.
    fn pattern_contains_name(
        pattern_node: tree_sitter::Node,
        name: &str,
        source: &[u8],
    ) -> bool {
        let mut cursor = pattern_node.walk();
        for child in pattern_node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    if node_text(child, source) == name {
                        return true;
                    }
                }
                "shorthand_property_identifier_pattern" => {
                    if node_text(child, source) == name {
                        return true;
                    }
                }
                "pair_pattern" => {
                    // { key: value } -- the binding is the value side
                    if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "identifier" && node_text(value, source) == name {
                            return true;
                        }
                        if value.kind() == "object_pattern" || value.kind() == "array_pattern" {
                            if Self::pattern_contains_name(value, name, source) {
                                return true;
                            }
                        }
                    }
                }
                "assignment_pattern" => {
                    // { a = default } -- the binding is the left side
                    if let Some(left) = child.child_by_field_name("left") {
                        if left.kind() == "identifier" && node_text(left, source) == name {
                            return true;
                        }
                        if left.kind() == "object_pattern" || left.kind() == "array_pattern" {
                            if Self::pattern_contains_name(left, name, source) {
                                return true;
                            }
                        }
                    }
                }
                "object_pattern" | "array_pattern" | "rest_pattern" => {
                    if Self::pattern_contains_name(child, name, source) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Check if an import statement declares the given name.
    fn import_declares_name(
        import_node: tree_sitter::Node,
        name: &str,
        source: &[u8],
    ) -> bool {
        let mut cursor = import_node.walk();
        // Use `children` (not `named_children`) to include all nodes in the grammar
        for child in import_node.children(&mut cursor) {
            if child.kind() == "import_clause" {
                if Self::import_clause_declares_name(child, name, source) {
                    return true;
                }
            }
        }
        false
    }

    fn import_clause_declares_name(
        clause: tree_sitter::Node,
        name: &str,
        source: &[u8],
    ) -> bool {
        let mut cursor = clause.walk();
        for child in clause.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    // default import: import React from 'react'
                    if node_text(child, source) == name {
                        return true;
                    }
                }
                "named_imports" => {
                    let mut inner = child.walk();
                    for spec in child.children(&mut inner) {
                        if spec.kind() == "import_specifier" {
                            // import { x as y } -- binding is `alias` field (y), or `name` if no alias
                            let binding = spec
                                .child_by_field_name("alias")
                                .or_else(|| spec.child_by_field_name("name"));
                            if let Some(b) = binding {
                                if b.kind() == "identifier" && node_text(b, source) == name {
                                    return true;
                                }
                            }
                        }
                    }
                }
                "namespace_import" => {
                    // import * as ns from 'mod'
                    let mut inner = child.walk();
                    for grandchild in child.children(&mut inner) {
                        if grandchild.kind() == "identifier"
                            && node_text(grandchild, source) == name
                        {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }
}

impl GraphPipeline for ImplicitGlobalsPipeline {
    fn name(&self) -> &str {
        "implicit_globals"
    }

    fn description(&self) -> &str {
        "Detects assignments to undeclared variables that create implicit globals"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree: &Tree = ctx.tree;
        let source: &[u8] = ctx.source;
        let file_path: &str = ctx.file_path;

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.assign_query, tree.root_node(), source);

        let lhs_idx = find_capture_index(&self.assign_query, "lhs");
        let assign_idx = find_capture_index(&self.assign_query, "assign");

        while let Some(m) = matches.next() {
            let lhs_cap = m.captures.iter().find(|c| c.index as usize == lhs_idx);
            let assign_cap = m.captures.iter().find(|c| c.index as usize == assign_idx);

            if let (Some(lhs), Some(assign)) = (lhs_cap, assign_cap) {
                // Only flag bare identifier assignments (not member_expression like obj.prop = ...)
                if lhs.node.kind() != "identifier" {
                    continue;
                }

                let name = node_text(lhs.node, source);

                if KNOWN_GLOBALS.contains(&name) {
                    continue;
                }

                if Self::has_declaration_in_scope(assign.node, name, source) {
                    continue;
                }

                if is_nolint_suppressed(source, assign.node, self.name()) {
                    continue;
                }

                let start = assign.node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "implicit_global".to_string(),
                    message: format!(
                        "assignment to `{name}` without declaration — creates an implicit global"
                    ),
                    snippet: extract_snippet(source, assign.node, 1),
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
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ImplicitGlobalsPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: "test.js",
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_implicit_global() {
        let src = "function foo() { x = 42; }";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "implicit_global");
        assert!(findings[0].message.contains("x"));
    }

    #[test]
    fn skips_declared_variable() {
        let src = "function foo() { let x; x = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_known_globals() {
        let src = "function foo() { module = {}; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_member_expression() {
        let src = "function foo() { obj.prop = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_parameter() {
        let src = "function foo(x) { x = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    // --- New tests for expanded declaration detection ---

    #[test]
    fn skips_function_declaration_as_binding() {
        let src = "function outer() { function inner() {} inner = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_class_declaration_as_binding() {
        let src = "function outer() { class Foo {} Foo = 42; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_import_specifier() {
        let src = "import { x } from 'mod';\nfunction f() { x = 1; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_import_default() {
        let src = "import React from 'react';\nfunction f() { React = null; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_import_namespace() {
        let src = "import * as utils from 'utils';\nfunction f() { utils = null; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_catch_parameter() {
        let src = "function f() { try {} catch(e) { e = null; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_destructured_parameter() {
        let src = "function f({a, b}) { a = 1; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_destructured_variable() {
        let src = "function f() { const {a, b} = obj; a = 1; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_expanded_known_globals() {
        let src = "function f() { JSON = null; Math = null; Promise = null; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn nolint_suppresses_finding() {
        let src = "function f() {\n// NOLINT(implicit_globals)\nx = 42;\n}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_for_of_binding() {
        let src = "function f() { for (const item of list) { item = 1; } }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
