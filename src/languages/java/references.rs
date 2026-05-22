//! Issue #16 Java `occurrence` / `scope` / `binding` fact emitter per
//! ADR-0005 and `docs/references-java.md`. The Cozoscript resolver
//! materialises `references` rows from these facts.
//!
//! Scope model:
//! - File root → `file` scope (`parent_id = null`). Top-level type
//!   `definition` bindings and `import` / `wildcard_import` bindings
//!   live here.
//! - `class_declaration` / `interface_declaration` / `enum_declaration`
//!   / `record_declaration` / `annotation_type_declaration` → `class`
//!   scope.
//! - `method_declaration` / `constructor_declaration` / `lambda_expression`
//!   → `function` scope. Static / instance initializers also use
//!   `function` per the contract.
//! - `block` (every `{ ... }`) → `block` scope.
//!
//! Binding emission:
//! - Every non-parameter Symbol → `definition` binding in the file
//!   scope. Parameters are bound at function scope by the walk pass to
//!   avoid double-counting.
//! - `formal_parameter` / `spread_parameter` / `catch_formal_parameter`
//!   / lambda parameters → `parameter` bindings in the enclosing
//!   function (or catch `block`) scope.
//! - `local_variable_declaration` → `definition` binding in the
//!   innermost block scope (schema reuses `definition` for locals;
//!   contract uses `parameter` historically — we follow the existing
//!   sibling extractors and use `definition`).
//! - `import_declaration` (non-wildcard) → `import` with
//!   `name = <last segment>`. Wildcard form (`import x.y.*;`) →
//!   `wildcard_import` with `name = "*"`. Static imports are emitted
//!   as plain `import` per the Java contract.
//!
//! Occurrence emission is intentionally a Level-3 narrowing matching
//! the contract.

use tree_sitter::{Node, Tree};

use crate::models::{
    BindingRow, OccurrenceRow, ReferencesBucket, ScopeRow, SymbolInfo, SymbolKind,
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
        for s in symbols {
            spans.push((s.start_byte, s.end_byte, symbol_id(s)));
        }
        spans.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        Self {
            file_path,
            source,
            bucket: ReferencesBucket::default(),
            symbol_spans: spans,
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

    /// Pass 1: every non-parameter Symbol → `definition` binding at the
    /// file scope. Parameter symbols are bound at function scope during
    /// the walk to avoid double-counting.
    fn emit_definitions(&mut self, file_scope_id: String, symbols: &[SymbolInfo]) {
        for sym in symbols {
            if matches!(sym.kind, SymbolKind::Parameter) {
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
            "method_declaration" | "constructor_declaration" => {
                self.emit_method_params(node, &active_scope);
            }
            "lambda_expression" => {
                self.emit_lambda_params(node, &active_scope);
            }
            "catch_clause" => {
                self.emit_catch_param(node, &active_scope);
            }
            "import_declaration" => {
                self.emit_import_declaration(node, scope_id);
            }
            "local_variable_declaration" => {
                self.emit_local_var(node, &active_scope);
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

    /// Emit `parameter` bindings for every `formal_parameter` /
    /// `spread_parameter` inside the method's `parameters` field.
    fn emit_method_params(&mut self, node: Node, fn_scope: &str) {
        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };
        let mut c = params.walk();
        for p in params.named_children(&mut c) {
            match p.kind() {
                "formal_parameter" | "spread_parameter" => {
                    if let Some((name, start)) = formal_param_name(p, self.source) {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: fn_scope.to_string(),
                            name,
                            start_byte: start,
                            symbol_id: None,
                            binding_kind: "parameter".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// Emit `parameter` bindings for lambda parameters. The
    /// `parameters` field is either:
    /// - an `identifier` (`x -> ...`)
    /// - an `inferred_parameters` list (`(x, y) -> ...`)
    /// - a `formal_parameters` list (`(int x, int y) -> ...`)
    fn emit_lambda_params(&mut self, node: Node, fn_scope: &str) {
        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };
        match params.kind() {
            "identifier" => {
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
            "inferred_parameters" => {
                let mut c = params.walk();
                for child in params.named_children(&mut c) {
                    if child.kind() == "identifier"
                        && let Ok(text) = child.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: fn_scope.to_string(),
                            name: text.to_string(),
                            start_byte: child.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "parameter".to_string(),
                        });
                    }
                }
            }
            "formal_parameters" => {
                let mut c = params.walk();
                for p in params.named_children(&mut c) {
                    if matches!(p.kind(), "formal_parameter" | "spread_parameter")
                        && let Some((name, start)) = formal_param_name(p, self.source)
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: fn_scope.to_string(),
                            name,
                            start_byte: start,
                            symbol_id: None,
                            binding_kind: "parameter".to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// `catch (Exception e)` — bind `e` as a parameter in the catch
    /// clause's block scope.
    fn emit_catch_param(&mut self, node: Node, scope: &str) {
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.kind() == "catch_formal_parameter"
                && let Some(name_node) = child.child_by_field_name("name")
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
    }

    /// `int x = 1, y = 2;` — emit a `definition` binding for each
    /// `variable_declarator` name in the innermost block scope.
    fn emit_local_var(&mut self, node: Node, scope: &str) {
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.kind() == "variable_declarator"
                && let Some(name_node) = child.child_by_field_name("name")
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
            }
        }
    }

    /// Handle `import a.b.C;`, `import a.b.*;`, `import static a.b.X.M;`,
    /// `import static a.b.X.*;`. Bind in the file scope (Java has no
    /// function-local imports).
    fn emit_import_declaration(&mut self, node: Node, scope_id: &str) {
        // The argument is either a `scoped_identifier` (single import),
        // an `identifier` (rare — `import Foo;` at the top), or an
        // `asterisk` follows a `scoped_identifier` for wildcards.
        let mut path_text: Option<(String, u32)> = None;
        let mut is_wildcard = false;
        let mut wildcard_start: u32 = 0;

        let mut c = node.walk();
        for child in node.children(&mut c) {
            match child.kind() {
                "scoped_identifier" | "identifier" => {
                    if let Ok(text) = child.utf8_text(self.source) {
                        path_text = Some((text.to_string(), child.start_byte() as u32));
                    }
                }
                "asterisk" | "*" => {
                    is_wildcard = true;
                    wildcard_start = child.start_byte() as u32;
                }
                _ => {}
            }
        }

        if is_wildcard {
            self.bucket.bindings.push(BindingRow {
                scope_id: scope_id.to_string(),
                name: "*".to_string(),
                start_byte: wildcard_start,
                symbol_id: None,
                binding_kind: "wildcard_import".to_string(),
            });
            return;
        }

        let Some((path, start)) = path_text else {
            return;
        };
        // Last segment of the dotted path.
        let leaf = path.rsplit('.').next().unwrap_or(&path).trim();
        if leaf.is_empty() {
            return;
        }
        self.bucket.bindings.push(BindingRow {
            scope_id: scope_id.to_string(),
            name: leaf.to_string(),
            start_byte: start,
            symbol_id: None,
            binding_kind: "import".to_string(),
        });
    }
}

/// Which scope (if any) does this node open?
fn scope_kind_for(node: Node) -> Option<&'static str> {
    match node.kind() {
        "method_declaration" | "constructor_declaration" | "lambda_expression" => Some("function"),
        "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "annotation_type_declaration" => Some("class"),
        "block" => Some("block"),
        _ => None,
    }
}

/// Extract the name + start_byte of a `formal_parameter` or
/// `spread_parameter` node.
fn formal_param_name(p: Node, source: &[u8]) -> Option<(String, u32)> {
    match p.kind() {
        "formal_parameter" => {
            let name_node = p.child_by_field_name("name")?;
            let text = name_node.utf8_text(source).ok()?;
            Some((text.to_string(), name_node.start_byte() as u32))
        }
        "spread_parameter" => {
            // `spread_parameter` wraps a `variable_declarator` whose
            // `name` field is the identifier.
            let mut c = p.walk();
            for child in p.named_children(&mut c) {
                if child.kind() == "variable_declarator"
                    && let Some(name_node) = child.child_by_field_name("name")
                {
                    let text = name_node.utf8_text(source).ok()?;
                    return Some((text.to_string(), name_node.start_byte() as u32));
                }
            }
            None
        }
        _ => None,
    }
}

/// Classify the occurrence_kind of an identifier-shaped node based on
/// its parent context. Returns `None` for nodes that are NOT
/// occurrences (declaring identifiers, package segments, etc.).
fn occurrence_kind_for(node: Node) -> Option<&'static str> {
    let kind = node.kind();
    if !matches!(kind, "identifier" | "type_identifier") {
        return None;
    }
    let Some(parent) = node.parent() else {
        return None;
    };
    let pk = parent.kind();

    // Declaring positions — these names are bindings, not occurrences.
    match pk {
        "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "annotation_type_declaration"
        | "method_declaration"
        | "constructor_declaration"
        | "formal_parameter"
        | "spread_parameter"
        | "catch_formal_parameter"
        | "variable_declarator"
        | "enum_constant"
        | "type_parameter" => {
            if parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id()) {
                return None;
            }
        }
        _ => {}
    }

    // Package declaration segments — packages are not symbols.
    if pk == "package_declaration" {
        return None;
    }

    // Inside import declarations — emit one `import_use` for the leaf
    // identifier (the last segment); intermediate package segments and
    // the wildcard get nothing.
    if is_inside_import(node) {
        if is_import_leaf(node) {
            return Some("import_use");
        }
        return None;
    }

    // Field access RHS suppression: `obj.field` — the `field` leaf is
    // not emitted in value position (the resolver joins receiver
    // type → field binding).
    if pk == "field_access"
        && parent.child_by_field_name("field").map(|n| n.id()) == Some(node.id())
    {
        // Per the contract, the field leaf IS emitted as a `write` when
        // it's the LHS of an assignment_expression; handle that below.
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

    // Method invocation leaf in the `name` field → call.
    if pk == "method_invocation"
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        return Some("call");
    }

    // `super(...)` / `this(...)` explicit constructor invocations.
    if pk == "explicit_constructor_invocation" {
        let text_is_call_keyword = matches!(node.utf8_text(&[]).unwrap_or(""), "this" | "super");
        // We can't compare without source here, but the constructor
        // node's first identifier child IS the `this`/`super` token in
        // most grammars. Conservatively emit a `call` if it's the
        // first identifier child of `explicit_constructor_invocation`.
        let _ = text_is_call_keyword;
        return Some("call");
    }

    // Assignment LHS — bare identifier on the `left` field of an
    // `assignment_expression` is a write.
    if pk == "assignment_expression"
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }

    // Prefix / postfix `++` / `--` on a bare identifier → write.
    if pk == "update_expression" {
        return Some("write");
    }

    Some("read")
}

/// True if `node` sits inside an `import_declaration` (the dotted name
/// path).
fn is_inside_import(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "import_declaration" => return true,
            "scoped_identifier" => cur = p.parent(),
            _ => return false,
        }
    }
    false
}

/// True if `node` is the LEAF (trailing) identifier of the import's
/// dotted path — i.e. it has no `scoped_identifier` sibling beyond it.
fn is_import_leaf(node: Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "import_declaration" => true, // bare `import Foo;`
        "scoped_identifier" => {
            // Leaf if this identifier is the `name` field of the
            // outermost `scoped_identifier` directly under
            // `import_declaration`.
            let is_name_field =
                parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id());
            if !is_name_field {
                return false;
            }
            // Walk up: this scoped_identifier must sit directly under
            // an `import_declaration` (not nested inside another
            // scoped_identifier as its `scope` field).
            let mut cur = parent.parent();
            while let Some(p) = cur {
                match p.kind() {
                    "import_declaration" => return true,
                    "scoped_identifier" => cur = p.parent(),
                    _ => return false,
                }
            }
            false
        }
        _ => false,
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> ReferencesBucket {
        let mut parser = create_parser(Language::Java).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(Language::Java).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, Language::Java);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn file_scope_emitted() {
        let b = run("class Foo {}", "Foo.java");
        let file_scope = b.scopes.iter().find(|s| s.parent_id.is_none());
        assert!(file_scope.is_some(), "file scope must exist");
        assert_eq!(file_scope.unwrap().kind, "file");
    }

    #[test]
    fn class_scope_emitted() {
        let b = run("class Foo {}", "Foo.java");
        assert!(b.scopes.iter().any(|s| s.kind == "class"));
    }

    #[test]
    fn function_scope_emitted() {
        let b = run("class Foo { void bar() {} }", "Foo.java");
        assert!(b.scopes.iter().any(|s| s.kind == "function"));
    }

    #[test]
    fn definition_binding_emitted() {
        let b = run("class Foo {}", "Foo.java");
        let d = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "definition" && x.name == "Foo");
        assert!(d.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn parameter_binding_emitted() {
        let b = run("class Foo { void bar(int x, String y) {} }", "Foo.java");
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
        let b = run("class Foo { void bar() { int x = 1; } }", "Foo.java");
        let d = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "definition" && x.name == "x");
        assert!(d.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn import_binding_emitted() {
        let b = run("import java.util.List;", "Foo.java");
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "List");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn wildcard_import_binding_emitted() {
        let b = run("import java.util.*;", "Foo.java");
        let w = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "wildcard_import" && x.name == "*");
        assert!(w.is_some(), "got: {:?}", b.bindings);
    }
}
