//! Issue #16 C `occurrence` / `scope` / `binding` fact emitter per
//! ADR-0005 and `docs/references-c.md`. The Cozoscript resolver
//! materialises `references` rows from these facts.
//!
//! Scope model (C has no class / namespace / module):
//! - File root → `file` scope (`parent_id = null`). Top-level
//!   declarations and `#include` wildcard imports bind here.
//! - `function_definition` → `function` scope. Parameters bind here.
//! - `compound_statement` → `block` scope. Local `declaration` rows
//!   bind in the innermost block.
//!
//! Binding emission:
//! - Every top-level Symbol (not Parameter, not block-scope Variable) →
//!   `definition` at file scope.
//! - `parameter_declaration` inside a function's parameter list →
//!   `parameter` binding in that function's scope.
//! - `declaration` inside a function body → `definition` binding in the
//!   innermost block scope (matches the local-variable Symbol from the
//!   symbol pipeline; resolver picks the innermost match).
//! - `preproc_include` directive → `wildcard_import` row (name = "*") at
//!   the file scope. `#include` exposes every name from the header; the
//!   resolver joins through the `imports` relation to enumerate them.
//!
//! Occurrence emission (Level 3): identifier-shaped nodes in non-declaring
//! position. Field leaves on `->` / `.` selectors are suppressed (field
//! policy). Compound assignments and `++` / `--` count as a single
//! `write`. Type-position identifiers (`type_identifier`) become
//! `type_use`.

use tree_sitter::{Node, Tree};

use crate::models::{BindingRow, OccurrenceRow, ReferencesBucket, ScopeRow, SymbolInfo, SymbolKind};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    let mut ctx = Ctx::new(file_path, source, symbols);
    let root = tree.root_node();
    let file_scope_id = ctx.push_scope(root, "file", None);
    ctx.emit_file_definitions(file_scope_id.clone(), symbols);
    ctx.walk(root, &file_scope_id);
    ctx.finish()
}

struct Ctx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    bucket: ReferencesBucket,
    /// Sorted `(start_byte, end_byte, symbol_id)` triples — used to find
    /// the innermost enclosing symbol of an occurrence.
    symbol_spans: Vec<(u32, u32, String)>,
    /// `(start_byte, end_byte)` spans of every Function symbol — used to
    /// detect whether a Parameter / Variable symbol is nested inside a
    /// function (so we can skip it in `emit_file_definitions` and let the
    /// walk re-emit at the right inner scope).
    function_spans: Vec<(u32, u32)>,
}

fn symbol_id(sym: &SymbolInfo) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        sym.file_path, sym.start_line, sym.start_column, sym.name, sym.kind
    )
}

impl<'a> Ctx<'a> {
    fn new(file_path: &'a str, source: &'a [u8], symbols: &[SymbolInfo]) -> Self {
        let mut spans = Vec::with_capacity(symbols.len());
        let mut function_spans = Vec::new();
        for s in symbols {
            spans.push((s.start_byte, s.end_byte, symbol_id(s)));
            if matches!(s.kind, SymbolKind::Function) {
                function_spans.push((s.start_byte, s.end_byte));
            }
        }
        spans.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        Self {
            file_path,
            source,
            bucket: ReferencesBucket::default(),
            symbol_spans: spans,
            function_spans,
        }
    }

    fn finish(self) -> ReferencesBucket {
        self.bucket
    }

    fn push_scope(&mut self, node: Node, kind: &str, parent: Option<&str>) -> String {
        let id = format!("{}|{}|{}", self.file_path, node.start_byte(), kind);
        self.bucket.scopes.push(ScopeRow {
            id: id.clone(),
            parent_id: parent.map(|s| s.to_string()),
            file_path: self.file_path.to_string(),
            kind: kind.to_string(),
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        });
        id
    }

    fn enclosing_symbol(&self, byte: u32) -> Option<&str> {
        let mut hit: Option<&str> = None;
        for (start, end, id) in &self.symbol_spans {
            if *start <= byte && byte <= *end {
                hit = Some(id.as_str());
            } else if *start > byte {
                break;
            }
        }
        hit
    }

    /// True if `byte` falls strictly inside any function's body span.
    /// Used to detect block-scope Variable / Parameter symbols so we
    /// skip them in pass 1.
    fn inside_function(&self, byte: u32) -> bool {
        self.function_spans
            .iter()
            .any(|(s, e)| *s < byte && byte < *e)
    }

    /// Pass 1: every top-level Symbol → `definition` at file scope.
    /// Parameters never live at file scope. Block-scope locals
    /// (Variable symbols whose start_byte is inside a function body)
    /// are handled by the walk pass at their innermost block scope.
    fn emit_file_definitions(&mut self, file_scope_id: String, symbols: &[SymbolInfo]) {
        for sym in symbols {
            if matches!(sym.kind, SymbolKind::Parameter) {
                continue;
            }
            if self.inside_function(sym.start_byte) {
                continue;
            }
            self.bucket.bindings.push(BindingRow {
                scope_id: file_scope_id.clone(),
                name: sym.name.clone(),
                start_byte: sym.start_byte,
                symbol_id: Some(symbol_id(sym)),
                binding_kind: "definition".to_string(),
            });
        }
    }

    /// Recursive walk. Tracks the active scope so occurrences land in
    /// the right `enclosing_scope_id`.
    fn walk(&mut self, node: Node, scope_id: &str) {
        let new_scope_kind = scope_kind_for(node);

        let active_scope = if let Some(kind) = new_scope_kind {
            self.push_scope(node, kind, Some(scope_id))
        } else {
            scope_id.to_string()
        };

        // Emit bindings unique to this node kind.
        match node.kind() {
            "function_definition" => {
                self.emit_function_params(node, &active_scope);
            }
            "preproc_include" => {
                self.emit_include(node, scope_id);
            }
            "declaration" => {
                // Local declarations inside a function body — emit
                // `definition` bindings in the innermost block scope.
                // File-scope declarations are already handled by
                // `emit_file_definitions`.
                if self.inside_function(node.start_byte() as u32) {
                    self.emit_local_declaration(node, &active_scope);
                }
            }
            _ => {}
        }

        // Occurrence emission.
        if let Some(kind) = occurrence_kind_for(node) {
            self.emit_occurrence(node, &active_scope, kind);
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, &active_scope);
        }
    }

    fn emit_occurrence(&mut self, node: Node, scope_id: &str, kind: &str) {
        let Ok(name) = node.utf8_text(self.source) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let start = node.start_byte() as u32;
        let id = format!("{}|{}|{}|{}", self.file_path, start, name, kind);
        let enclosing = self.enclosing_symbol(start).map(|s| s.to_string());
        self.bucket.occurrences.push(OccurrenceRow {
            id,
            name: name.to_string(),
            file_path: self.file_path.to_string(),
            start_byte: start,
            end_byte: node.end_byte() as u32,
            enclosing_symbol_id: enclosing,
            enclosing_scope_id: scope_id.to_string(),
            occurrence_kind: kind.to_string(),
        });
    }

    /// Emit `parameter` bindings for every `parameter_declaration` in
    /// the function's `parameter_list`. The C grammar nests
    /// `parameter_list` inside `function_declarator`, which may itself
    /// sit under one or more `pointer_declarator` layers (for functions
    /// returning pointers).
    fn emit_function_params(&mut self, node: Node, fn_scope: &str) {
        let Some(decl) = node.child_by_field_name("declarator") else {
            return;
        };
        let Some(fdecl) = find_function_declarator(decl) else {
            return;
        };
        let Some(params) = fdecl.child_by_field_name("parameters") else {
            return;
        };
        let mut c = params.walk();
        for p in params.named_children(&mut c) {
            if p.kind() != "parameter_declaration" {
                continue;
            }
            // Skip parameter_declaration whose declarator is absent
            // (e.g. `void f(int)` — unnamed parameter).
            let Some(pdecl) = p.child_by_field_name("declarator") else {
                continue;
            };
            let Some(name_node) = find_innermost_identifier(pdecl) else {
                continue;
            };
            let Ok(text) = name_node.utf8_text(self.source) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            self.bucket.bindings.push(BindingRow {
                scope_id: fn_scope.to_string(),
                name: text.to_string(),
                start_byte: name_node.start_byte() as u32,
                symbol_id: None,
                binding_kind: "parameter".to_string(),
            });
        }
    }

    /// `int x = 1, *p, arr[10];` — emit one `definition` per declared
    /// name in the innermost block scope. The C grammar exposes each
    /// declared item via the `declarator` field (single name) or via
    /// multiple children that are `init_declarator` / identifier /
    /// pointer_declarator / array_declarator nodes.
    fn emit_local_declaration(&mut self, node: Node, scope: &str) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            // The type specifier is also a child; skip anything that
            // can't be a declarator. The declarator-like node kinds:
            match child.kind() {
                "identifier"
                | "init_declarator"
                | "pointer_declarator"
                | "array_declarator"
                | "function_declarator" => {}
                _ => continue,
            }
            // Skip function prototypes inside a function body — they
            // declare a function (a definition at file scope per the
            // contract); for the local-binding pass we only emit
            // variables.
            if has_function_declarator(child) {
                continue;
            }
            let Some(name_node) = find_innermost_identifier(child) else {
                continue;
            };
            let Ok(text) = name_node.utf8_text(self.source) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            self.bucket.bindings.push(BindingRow {
                scope_id: scope.to_string(),
                name: text.to_string(),
                start_byte: name_node.start_byte() as u32,
                symbol_id: None,
                binding_kind: "definition".to_string(),
            });
        }
    }

    /// `#include <foo.h>` / `#include "bar.h"` → `wildcard_import` at
    /// the file scope. C has no per-symbol import — the header brings
    /// every name with it.
    fn emit_include(&mut self, node: Node, scope_id: &str) {
        self.bucket.bindings.push(BindingRow {
            scope_id: scope_id.to_string(),
            name: "*".to_string(),
            start_byte: node.start_byte() as u32,
            symbol_id: None,
            binding_kind: "wildcard_import".to_string(),
        });
    }
}

/// Which scope (if any) does this node open?
fn scope_kind_for(node: Node) -> Option<&'static str> {
    match node.kind() {
        "function_definition" => Some("function"),
        "compound_statement" => Some("block"),
        _ => None,
    }
}

/// Walk down a declarator chain (`pointer_declarator`, `array_declarator`,
/// `init_declarator`, `function_declarator`, `parenthesized_declarator`)
/// to the innermost `identifier` / `field_identifier`.
fn find_innermost_identifier(node: Node) -> Option<Node> {
    match node.kind() {
        "identifier" | "field_identifier" => Some(node),
        _ => {
            // Prefer the `declarator` field if present.
            if let Some(inner) = node.child_by_field_name("declarator")
                && let Some(found) = find_innermost_identifier(inner)
            {
                return Some(found);
            }
            // Fallback: scan named children.
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                if let Some(found) = find_innermost_identifier(child) {
                    return Some(found);
                }
            }
            None
        }
    }
}

/// Find a `function_declarator` inside a declarator chain (handles
/// `int *foo(...)` where the function_declarator sits under a
/// `pointer_declarator`).
fn find_function_declarator(node: Node) -> Option<Node> {
    if node.kind() == "function_declarator" {
        return Some(node);
    }
    if let Some(inner) = node.child_by_field_name("declarator")
        && let Some(found) = find_function_declarator(inner)
    {
        return Some(found);
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        if let Some(found) = find_function_declarator(child) {
            return Some(found);
        }
    }
    None
}

/// True if `node` contains a `function_declarator` (used to skip
/// nested function prototypes when emitting local variable bindings).
fn has_function_declarator(node: Node) -> bool {
    if node.kind() == "function_declarator" {
        return true;
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        if has_function_declarator(child) {
            return true;
        }
    }
    false
}

/// True if `node` is the defining identifier of its parent (the
/// declarator chain inside a function_definition / declaration /
/// parameter_declaration / struct_specifier / etc.).
fn is_defining_identifier(node: Node) -> bool {
    // Walk parents until we hit a known declaration-introducing node.
    // If we pass through only `declarator`-flavoured nodes along the
    // way, this identifier is the defined name.
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "function_definition"
            | "declaration"
            | "parameter_declaration"
            | "field_declaration"
            | "type_definition"
            | "struct_specifier"
            | "union_specifier"
            | "enum_specifier"
            | "preproc_def"
            | "preproc_function_def"
            | "enumerator" => return true,
            "pointer_declarator"
            | "array_declarator"
            | "function_declarator"
            | "init_declarator"
            | "parenthesized_declarator" => {
                cur = p.parent();
            }
            _ => return false,
        }
    }
    false
}

/// Classify the occurrence_kind of an identifier-shaped node based on
/// its parent context. Returns `None` for nodes that are NOT
/// occurrences (declaring identifiers, include paths, field leaves of
/// `->` / `.` selectors in value position).
fn occurrence_kind_for(node: Node) -> Option<&'static str> {
    let kind = node.kind();
    if !matches!(
        kind,
        "identifier" | "type_identifier" | "field_identifier"
    ) {
        return None;
    }
    let Some(parent) = node.parent() else {
        return None;
    };
    let pk = parent.kind();

    // Defining identifiers — not occurrences.
    if is_defining_identifier(node) {
        return None;
    }

    // Include paths are not identifiers in the grammar (they're
    // string_literal / system_lib_string), but guard anyway.
    if pk == "preproc_include" {
        return None;
    }

    // Field leaves on `s.field` / `p->field` are suppressed in value
    // position (field-row policy). Promote to `write` if this whole
    // field_expression sits on the LHS of an assignment.
    if pk == "field_expression"
        && parent.child_by_field_name("field").map(|n| n.id()) == Some(node.id())
    {
        if let Some(grand) = parent.parent()
            && grand.kind() == "assignment_expression"
            && grand.child_by_field_name("left").map(|n| n.id()) == Some(parent.id())
        {
            return Some("write");
        }
        return None;
    }

    // type_identifier always denotes a type use.
    if kind == "type_identifier" {
        return Some("type_use");
    }

    // Call: identifier in the `function` field of a `call_expression`.
    // Indirect / expression callees (`a[i](x)`, `(funcs[i])(x)`) don't
    // place a bare identifier there, so this fires only for named
    // callees (including function-pointer parameters).
    if pk == "call_expression"
        && parent.child_by_field_name("function").map(|n| n.id()) == Some(node.id())
    {
        return Some("call");
    }

    // Bare identifier LHS of an assignment_expression → write.
    if pk == "assignment_expression"
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }

    // `x++` / `x--` / `++x` / `--x` → write.
    if pk == "update_expression" {
        return Some("write");
    }

    Some("read")
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> ReferencesBucket {
        let mut parser = create_parser(Language::C).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(Language::C).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, Language::C);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn file_scope_emitted() {
        let b = run("int main(void) { return 0; }", "main.c");
        let file_scope = b.scopes.iter().find(|s| s.parent_id.is_none());
        assert!(file_scope.is_some(), "file scope must exist");
        assert_eq!(file_scope.unwrap().kind, "file");
    }

    #[test]
    fn function_scope_emitted() {
        let b = run("int main(void) { return 0; }", "main.c");
        assert!(
            b.scopes.iter().any(|s| s.kind == "function"),
            "got: {:?}",
            b.scopes
        );
    }

    #[test]
    fn block_scope_emitted() {
        let b = run(
            "void f(void) { { int x = 1; } }",
            "main.c",
        );
        // At least two block scopes: the function body and the nested
        // `{ ... }` block.
        let blocks = b.scopes.iter().filter(|s| s.kind == "block").count();
        assert!(blocks >= 2, "expected >=2 block scopes, got: {:?}", b.scopes);
    }

    #[test]
    fn parameter_binding_emitted() {
        let b = run(
            "int add(int a, int b) { return a + b; }",
            "main.c",
        );
        let names: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .map(|x| x.name.as_str())
            .collect();
        assert!(names.contains(&"a"), "got: {:?}", b.bindings);
        assert!(names.contains(&"b"), "got: {:?}", b.bindings);
    }

    #[test]
    fn pointer_parameter_binding_emitted() {
        let b = run(
            "void set(int *p, char **argv) { }",
            "main.c",
        );
        let names: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .map(|x| x.name.as_str())
            .collect();
        assert!(names.contains(&"p"), "got: {:?}", b.bindings);
        assert!(names.contains(&"argv"), "got: {:?}", b.bindings);
    }

    #[test]
    fn unnamed_parameter_skipped() {
        let b = run("void f(int) { }", "main.c");
        let params = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .count();
        assert_eq!(params, 0, "unnamed param should produce no binding, got: {:?}", b.bindings);
    }

    #[test]
    fn local_var_binding_emitted() {
        let b = run(
            "int main(void) { int x = 1; int y; return x + y; }",
            "main.c",
        );
        // Locals bind in the innermost block scope, not file scope.
        let xs: Vec<&BindingRow> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "definition" && x.name == "x")
            .collect();
        let ys: Vec<&BindingRow> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "definition" && x.name == "y")
            .collect();
        assert!(!xs.is_empty(), "local `x` definition missing, got: {:?}", b.bindings);
        assert!(!ys.is_empty(), "local `y` definition missing, got: {:?}", b.bindings);
        // The binding must point at a block scope, not the file scope.
        let block_scope_ids: std::collections::HashSet<&str> = b
            .scopes
            .iter()
            .filter(|s| s.kind == "block")
            .map(|s| s.id.as_str())
            .collect();
        assert!(
            xs.iter().any(|x| block_scope_ids.contains(x.scope_id.as_str())),
            "`x` local must bind at a block scope, got scope_ids: {:?}",
            xs.iter().map(|x| &x.scope_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn file_scope_definition_binding() {
        let b = run("int counter = 0;", "main.c");
        let d = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "definition" && x.name == "counter");
        assert!(d.is_some(), "got: {:?}", b.bindings);
        // Must live at the file scope.
        let file_scope_id = b
            .scopes
            .iter()
            .find(|s| s.parent_id.is_none())
            .map(|s| s.id.clone())
            .expect("file scope");
        assert_eq!(d.unwrap().scope_id, file_scope_id);
    }

    #[test]
    fn include_emits_wildcard_import_binding() {
        let b = run("#include <stdio.h>\nint main(void) { return 0; }", "main.c");
        let w = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "wildcard_import" && x.name == "*");
        assert!(w.is_some(), "got: {:?}", b.bindings);
        // Must live at the file scope.
        let file_scope_id = b
            .scopes
            .iter()
            .find(|s| s.parent_id.is_none())
            .map(|s| s.id.clone())
            .expect("file scope");
        assert_eq!(w.unwrap().scope_id, file_scope_id);
    }

    #[test]
    fn local_include_also_emits_wildcard() {
        let b = run("#include \"myheader.h\"\n", "main.c");
        let w = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "wildcard_import" && x.name == "*");
        assert!(w.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn call_occurrence_emitted() {
        let b = run("void g(void); void f(void) { g(); }", "main.c");
        let c = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "call" && o.name == "g");
        assert!(c.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn assignment_write_occurrence_emitted() {
        let b = run(
            "int counter = 0; void bump(void) { counter = 1; }",
            "main.c",
        );
        let w = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "write" && o.name == "counter");
        assert!(w.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn defining_function_name_not_an_occurrence() {
        let b = run("int main(void) { return 0; }", "main.c");
        // `main` should appear only as a definition binding, not as a
        // read/call occurrence.
        let read_main = b
            .occurrences
            .iter()
            .any(|o| o.name == "main" && o.occurrence_kind != "call");
        assert!(!read_main, "main should not emit a non-call occurrence, got: {:?}", b.occurrences);
    }
}
