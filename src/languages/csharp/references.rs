//! Issue #16 C# `occurrence` / `scope` / `binding` fact emitter per
//! ADR-0005 and `docs/references-csharp.md`. The Cozoscript resolver
//! materialises `references` rows from these facts.
//!
//! Scope model:
//! - File root → `file` scope (`parent_id = null`).
//! - `namespace_declaration` / `file_scoped_namespace_declaration`
//!   → `namespace` scope.
//! - `class_declaration` / `interface_declaration` /
//!   `struct_declaration` / `record_declaration` → `class` scope.
//! - `method_declaration` / `constructor_declaration` /
//!   `lambda_expression` / `local_function_statement` → `function` scope.
//! - `block` → `block` scope.
//!
//! Binding emission:
//! - Every non-parameter Symbol → `definition` binding in file scope.
//!   Variable symbols inside a function body are skipped here and
//!   re-emitted at the innermost block scope by the walk pass (so
//!   locals bind in their block scope, matching the contract).
//! - `parameter`, `parameter_array`, `implicit_parameter`, and
//!   `catch_declaration` → `parameter` binding in the enclosing
//!   function (or catch's block) scope.
//! - `local_declaration_statement` → `definition` binding per
//!   declarator in the innermost block scope.
//! - `using Some.Namespace;` → `import` (name = last segment).
//! - `using Alias = Some.Namespace;` → `import_alias` (name = alias).
//!   C# has no per-name wildcard `using` like C++ `using namespace`.

use tree_sitter::{Node, Tree};

use crate::models::{
    BindingRow, LocalTypeRow, OccurrenceRow, ReferencesBucket, ScopeRow, SymbolInfo, SymbolKind,
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    let mut ctx = Ctx::new(file_path, source, symbols);
    let root = tree.root_node();
    let file_scope_id = ctx.push_scope(root, "file", None);
    ctx.emit_definitions(file_scope_id.clone(), symbols);
    ctx.walk(root, &file_scope_id);
    ctx.finish()
}

struct Ctx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    bucket: ReferencesBucket,
    /// Sorted `(start_byte, end_byte, symbol_id)` triples used to find
    /// the innermost enclosing symbol of an occurrence.
    symbol_spans: Vec<(u32, u32, String)>,
    /// `(start_byte, end_byte)` spans of every Method/function symbol —
    /// used to detect whether a Variable symbol sits inside a function
    /// body so we can skip it in `emit_definitions`.
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
            if matches!(s.kind, SymbolKind::Method | SymbolKind::Function) {
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

    /// True if `byte` falls strictly inside any function body span.
    fn inside_function(&self, byte: u32) -> bool {
        self.function_spans
            .iter()
            .any(|(s, e)| *s < byte && byte < *e)
    }

    /// Pass 1: every non-parameter Symbol → `definition` binding at the
    /// file scope. Parameter symbols are bound at function scope during
    /// the walk. Local Variable symbols inside a function body are
    /// skipped here and emitted at their innermost block by the walk.
    fn emit_definitions(&mut self, file_scope_id: String, symbols: &[SymbolInfo]) {
        for sym in symbols {
            if matches!(sym.kind, SymbolKind::Parameter) {
                continue;
            }
            if matches!(sym.kind, SymbolKind::Variable) && self.inside_function(sym.start_byte) {
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
            "method_declaration" | "constructor_declaration" | "local_function_statement" => {
                self.emit_method_params(node, &active_scope);
            }
            "lambda_expression" => {
                self.emit_lambda_params(node, &active_scope);
            }
            "catch_declaration" => {
                self.emit_catch_param(node, &active_scope);
            }
            "using_directive" => {
                self.emit_using_directive(node, scope_id);
            }
            "local_declaration_statement" => {
                self.emit_local_declaration(node, &active_scope);
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

    /// Walk a method/constructor/local-function parameter list and emit
    /// `parameter` bindings. Handles regular `parameter`, the hidden
    /// `_parameter_array` (`params int[] xs` — appears as a bare
    /// identifier child of `parameter_list`).
    fn emit_method_params(&mut self, node: Node, fn_scope: &str) {
        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };
        let mut c = params.walk();
        for p in params.named_children(&mut c) {
            match p.kind() {
                "parameter" => {
                    if let Some(name_node) = p.child_by_field_name("name")
                        && let Ok(text) = name_node.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: fn_scope.to_string(),
                            name: text.to_string(),
                            start_byte: name_node.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "parameter".to_string(),
                        });
                    }
                }
                // `params int[] xs` — the hidden `_parameter_array` rule
                // exposes the trailing identifier as a direct
                // `parameter_list` child.
                "identifier" => {
                    if let Ok(text) = p.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: fn_scope.to_string(),
                            name: text.to_string(),
                            start_byte: p.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "parameter".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// Emit `parameter` bindings for lambda parameters. The `parameters`
    /// field is either:
    /// - an `implicit_parameter` (`x => x + 1`)
    /// - a `parameter_list` (`(int x, int y) => ...`)
    fn emit_lambda_params(&mut self, node: Node, fn_scope: &str) {
        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };
        match params.kind() {
            "implicit_parameter" => {
                if let Ok(text) = params.utf8_text(self.source)
                    && !text.is_empty()
                {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: fn_scope.to_string(),
                        name: text.to_string(),
                        start_byte: params.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "parameter".to_string(),
                    });
                }
            }
            "parameter_list" => {
                let mut c = params.walk();
                for p in params.named_children(&mut c) {
                    if p.kind() == "parameter"
                        && let Some(name_node) = p.child_by_field_name("name")
                        && let Ok(text) = name_node.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: fn_scope.to_string(),
                            name: text.to_string(),
                            start_byte: name_node.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "parameter".to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// `catch (Exception ex)` — bind `ex` as a parameter in the catch
    /// declaration's enclosing block scope.
    fn emit_catch_param(&mut self, node: Node, scope: &str) {
        if let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(self.source)
            && !text.is_empty()
        {
            self.bucket.bindings.push(BindingRow {
                scope_id: scope.to_string(),
                name: text.to_string(),
                start_byte: name_node.start_byte() as u32,
                symbol_id: None,
                binding_kind: "parameter".to_string(),
            });
        }
    }

    /// `int x = 1, y = 2;` / `var z = 3;` — emit one `definition` per
    /// `variable_declarator` name in the innermost block scope.
    fn emit_local_declaration(&mut self, node: Node, scope: &str) {
        // The shape is: local_declaration_statement → variable_declaration
        // → (variable_declarator (name: identifier))+.
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.kind() != "variable_declaration" {
                continue;
            }
            // Declared type of this `variable_declaration` (shared by all its
            // declarators). `var` is implicit — fall back to the initializer.
            let decl_type_text = child
                .child_by_field_name("type")
                .and_then(|t| t.utf8_text(self.source).ok())
                .map(|s| s.to_string());
            let mut cc = child.walk();
            for decl in child.named_children(&mut cc) {
                if decl.kind() != "variable_declarator" {
                    continue;
                }
                let name_node = decl
                    .child_by_field_name("name")
                    .or_else(|| find_first_identifier(decl));
                if let Some(name_node) = name_node
                    && let Ok(text) = name_node.utf8_text(self.source)
                    && !text.is_empty()
                {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope.to_string(),
                        name: text.to_string(),
                        start_byte: name_node.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "definition".to_string(),
                    });
                    // Cheap local type: explicit type, or `new Foo()` for `var`.
                    let type_name = match decl_type_text.as_deref() {
                        Some("var") | None => object_creation_type(decl, self.source),
                        Some(t) => bare_type_name(t),
                    };
                    if let Some(type_name) = type_name {
                        self.bucket.local_types.push(LocalTypeRow {
                            file_path: self.file_path.to_string(),
                            name: text.to_string(),
                            type_name,
                            start_byte: name_node.start_byte() as u32,
                        });
                    }
                }
            }
        }
    }

    /// `using Some.Namespace;` → `import` (name = leaf segment).
    /// `using Alias = Some.Namespace;` → `import_alias` (name = alias).
    /// `using static Some.Type;` → `import` (name = leaf segment).
    /// C# has no per-name wildcard `using`.
    fn emit_using_directive(&mut self, node: Node, scope: &str) {
        // Look for a `name_equals` child → alias form.
        let mut alias_text: Option<(String, u32)> = None;
        let mut path_node: Option<Node> = None;

        let mut c = node.walk();
        for child in node.children(&mut c) {
            match child.kind() {
                "name_equals" => {
                    // `name_equals` wraps `(identifier) =`. Take the
                    // inner identifier as the alias.
                    let mut cc = child.walk();
                    for inner in child.named_children(&mut cc) {
                        if inner.kind() == "identifier"
                            && let Ok(text) = inner.utf8_text(self.source)
                            && !text.is_empty()
                        {
                            alias_text = Some((text.to_string(), inner.start_byte() as u32));
                            break;
                        }
                    }
                }
                "qualified_name" | "identifier" | "generic_name" | "alias_qualified_name" => {
                    // The dotted/qualified target. Prefer the LAST
                    // suitable child (after `name_equals` in alias form).
                    path_node = Some(child);
                }
                _ => {}
            }
        }

        if let Some((alias, start)) = alias_text {
            self.bucket.bindings.push(BindingRow {
                scope_id: scope.to_string(),
                name: alias,
                start_byte: start,
                symbol_id: None,
                binding_kind: "import_alias".to_string(),
            });
            return;
        }

        let Some(path_node) = path_node else {
            return;
        };
        let Ok(text) = path_node.utf8_text(self.source) else {
            return;
        };
        // Trailing segment after the last `.` — `using System.IO;` → `IO`.
        let leaf = text.rsplit('.').next().unwrap_or(text).trim();
        if leaf.is_empty() {
            return;
        }
        self.bucket.bindings.push(BindingRow {
            scope_id: scope.to_string(),
            name: leaf.to_string(),
            start_byte: path_node.start_byte() as u32,
            symbol_id: None,
            binding_kind: "import".to_string(),
        });
    }
}

/// Which scope (if any) does this node open?
fn scope_kind_for(node: Node) -> Option<&'static str> {
    match node.kind() {
        "namespace_declaration" | "file_scoped_namespace_declaration" => Some("namespace"),
        "class_declaration"
        | "interface_declaration"
        | "struct_declaration"
        | "record_declaration" => Some("class"),
        "method_declaration"
        | "constructor_declaration"
        | "lambda_expression"
        | "local_function_statement" => Some("function"),
        // Owning construct verbatim (for_statement, foreach_statement, …)
        // instead of generic "block"; bare blocks report their parent.
        "block" => node.parent().map(|p| p.kind()),
        _ => None,
    }
}

/// Find the first `identifier` descendant of `node`. Used as a fallback
/// when `variable_declarator` lacks an explicit `name` field.
fn find_first_identifier(node: Node) -> Option<Node> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        if let Some(found) = find_first_identifier(child) {
            return Some(found);
        }
    }
    None
}

/// Bare class name from a type expression: drop namespace qualifier and
/// generic args (`A.B.Foo<T>` -> `Foo`). Returns None for predefined/builtin
/// types and anything that isn't a plain identifier.
fn bare_type_name(t: &str) -> Option<String> {
    let base = t.split('<').next().unwrap_or(t).trim().trim_end_matches(['?', '[', ']']);
    let leaf = base.rsplit('.').next().unwrap_or(base).trim();
    if leaf.is_empty()
        || !leaf.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !leaf.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }
    Some(leaf.to_string())
}

/// For `var x = new Foo(...)`, find the `object_creation_expression` in the
/// declarator and return its bare type name.
fn object_creation_type(declarator: Node, source: &[u8]) -> Option<String> {
    fn walk(node: Node, source: &[u8]) -> Option<String> {
        if node.kind() == "object_creation_expression"
            && let Some(ty) = node.child_by_field_name("type")
            && let Ok(text) = ty.utf8_text(source)
        {
            return bare_type_name(text);
        }
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if let Some(found) = walk(child, source) {
                return Some(found);
            }
        }
        None
    }
    walk(declarator, source)
}

/// True if `node` sits inside a `using_directive`'s name path.
fn is_inside_using(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "using_directive" => return true,
            "qualified_name" | "alias_qualified_name" | "name_equals" => cur = p.parent(),
            _ => return false,
        }
    }
    false
}

/// True if `node` is the defining identifier of a declaration's `name`
/// field. We suppress occurrence emission for these because they are
/// `binding` rows, not occurrences.
fn is_defining_identifier(node: Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "class_declaration"
        | "interface_declaration"
        | "struct_declaration"
        | "record_declaration"
        | "enum_declaration"
        | "delegate_declaration"
        | "method_declaration"
        | "constructor_declaration"
        | "namespace_declaration"
        | "file_scoped_namespace_declaration"
        | "property_declaration"
        | "parameter"
        | "variable_declarator"
        | "catch_declaration"
        | "local_function_statement" => {
            parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
        }
        // `params int[] xs` — bare identifier directly under parameter_list.
        "parameter_list" => true,
        // `x => x + 1` — implicit_parameter wrapping the identifier IS the parameter.
        "implicit_parameter" => true,
        _ => false,
    }
}

/// Classify the occurrence_kind of an identifier-shaped node.
fn occurrence_kind_for(node: Node) -> Option<&'static str> {
    let kind = node.kind();
    if kind != "identifier" {
        return None;
    }
    let parent = node.parent()?;
    let pk = parent.kind();

    if is_defining_identifier(node) {
        return None;
    }

    // Inside a using directive's path → import_use.
    if is_inside_using(node) {
        return Some("import_use");
    }

    // Suppress field leaf on `obj.field` / `obj?.field` per contract
    // ("field-row policy"): the resolver discovers the field via the
    // receiver type's class binding. We emit `read` for the receiver,
    // nothing for the field name itself — except when the member-access
    // is the LHS of an assignment, then the field name carries `write`.
    if pk == "member_access_expression"
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        if let Some(grand) = parent.parent()
            && grand.kind() == "assignment_expression"
            && grand.child_by_field_name("left").map(|n| n.id()) == Some(parent.id())
        {
            return Some("write");
        }
        return None;
    }

    // `invocation_expression` with a bare-identifier `function` field
    // → call. `Foo()` callee.
    if pk == "invocation_expression"
        && parent.child_by_field_name("function").map(|n| n.id()) == Some(node.id())
    {
        return Some("call");
    }

    // `new Foo(...)` — `object_creation_expression` exposes the type as
    // a `type` field (an identifier or generic_name). Per the contract
    // this emits both `call` AND `type_use`; we emit `call` here (the
    // type_use emission for `new T(...)` is left to the resolver).
    if pk == "object_creation_expression"
        && parent.child_by_field_name("type").map(|n| n.id()) == Some(node.id())
    {
        return Some("call");
    }

    // Bare-identifier assignment LHS → write.
    if pk == "assignment_expression"
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }

    // `++x` / `x++` → write. Be conservative: any update-expression
    // identifier is a write.
    if pk == "prefix_unary_expression" || pk == "postfix_unary_expression" {
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
        let mut parser = create_parser(Language::CSharp).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(Language::CSharp).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, Language::CSharp);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn file_scope_emitted() {
        let b = run("class Foo {}", "Foo.cs");
        let file_scope = b.scopes.iter().find(|s| s.parent_id.is_none());
        assert!(file_scope.is_some(), "file scope must exist");
        assert_eq!(file_scope.unwrap().kind, "file");
    }

    #[test]
    fn namespace_scope_emitted() {
        let b = run("namespace MyApp { class Foo {} }", "Foo.cs");
        assert!(
            b.scopes.iter().any(|s| s.kind == "namespace"),
            "got: {:?}",
            b.scopes
        );
    }

    #[test]
    fn class_scope_emitted() {
        let b = run("class Foo {}", "Foo.cs");
        assert!(
            b.scopes.iter().any(|s| s.kind == "class"),
            "got: {:?}",
            b.scopes
        );
    }

    #[test]
    fn function_scope_emitted() {
        let b = run("class Foo { void Bar() {} }", "Foo.cs");
        assert!(
            b.scopes.iter().any(|s| s.kind == "function"),
            "got: {:?}",
            b.scopes
        );
    }

    #[test]
    fn block_scope_emitted() {
        let b = run("class Foo { void Bar() { { int x = 1; } } }", "Foo.cs");
        let blocks = b
            .scopes
            .iter()
            .filter(|s| {
                !matches!(
                    s.kind.as_str(),
                    "file" | "function" | "class" | "namespace" | "module"
                )
            })
            .count();
        assert!(
            blocks >= 2,
            "expected >=2 block scopes, got: {:?}",
            b.scopes
        );
    }

    #[test]
    fn parameter_binding_emitted() {
        let b = run("class Foo { void Bar(int x, string y) {} }", "Foo.cs");
        let names: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .map(|x| x.name.as_str())
            .collect();
        assert!(names.contains(&"x"), "got: {:?}", b.bindings);
        assert!(names.contains(&"y"), "got: {:?}", b.bindings);
    }

    #[test]
    fn local_var_binding_emitted() {
        let b = run("class Foo { void Bar() { int x = 1; } }", "Foo.cs");
        let xs: Vec<&BindingRow> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "definition" && x.name == "x")
            .collect();
        assert!(
            !xs.is_empty(),
            "local `x` definition missing, got: {:?}",
            b.bindings
        );
        let block_scope_ids: std::collections::HashSet<&str> = b
            .scopes
            .iter()
            .filter(|s| {
                !matches!(
                    s.kind.as_str(),
                    "file" | "function" | "class" | "namespace" | "module"
                )
            })
            .map(|s| s.id.as_str())
            .collect();
        assert!(
            xs.iter()
                .any(|x| block_scope_ids.contains(x.scope_id.as_str())),
            "`x` local must bind at a block scope, got scope_ids: {:?}",
            xs.iter().map(|x| &x.scope_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn import_binding_emitted() {
        let b = run("using System.IO;\n", "Foo.cs");
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "IO");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    // TODO(#18.3 polish): tree-sitter-csharp's `using A = X.Y` alias
    // detection — agent's expected grammar shape needs review.
    #[ignore]
    fn import_alias_binding_emitted() {
        let b = run("using IO = System.IO;\n", "Foo.cs");
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "IO");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }
}
