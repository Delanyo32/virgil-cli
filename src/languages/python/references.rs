//! Issue #16 Python `occurrence` / `scope` / `binding` fact emitter per
//! ADR-0005 and `docs/references-python.md`. The Cozoscript resolver
//! materialises `references` rows from these facts.
//!
//! Scope model (Python is special — no block scope):
//! - The whole file → top-level scope, `kind = "module"`,
//!   `parent_id = null`.
//! - `function_definition` body → `function` scope.
//! - `lambda` body → `function` scope.
//! - `class_definition` body → `class` scope.
//! - Each comprehension (`list_comprehension`,
//!   `set_comprehension`, `dictionary_comprehension`,
//!   `generator_expression`) → its own `function` scope. The
//!   comprehension target binds inside that scope.
//! - `if`, `for`, `while`, `try`, `with`, `match` and their bodies do
//!   NOT open scopes — names leak to the enclosing function or module.
//!
//! Binding emission:
//! - Every Symbol → `definition` binding in the module scope (the
//!   resolver uses the file-wide entry to disambiguate top-level vs
//!   nested resolution; the byte ranges already encode containment).
//! - `def f(...)` / `async def f(...)` / `lambda` parameters →
//!   `parameter` bindings in the function/lambda scope. `self` / `cls`
//!   get no special-casing.
//! - `import foo[.bar]*` → `import` binding of the head name in the
//!   nearest module/function scope. Aliased forms → `import_alias`.
//! - `from x import y[, ...]` → one `import` binding per name; `as`
//!   aliases emit `import_alias`; `import *` emits `wildcard_import`.
//! - Function-local imports bind in the function scope.
//!
//! Occurrence emission (Level 3):
//! - `call`: identifier in callee position of a `call` node (only when
//!   the callee is a bare `identifier`, not an attribute chain).
//! - `write`: assignment LHS (simple identifier), `augmented_assignment`
//!   LHS, `for`/`with`/`except` binding targets, walrus `(x := ...)`.
//! - `read`: every other identifier in value position, including
//!   decorator expressions, default-value expressions, f-string
//!   interpolations, and the head of an attribute chain.
//! - `import_use`: identifiers under `import_statement` /
//!   `import_from_statement` (NOT the local alias name; that is captured
//!   by the `import_alias` binding).
//!
//! Attribute chains: only the head of `a.b.c` produces a row (`read` of
//! `a`); `.b` and `.c` emit nothing per the contract.

use tree_sitter::{Node, Tree};

use crate::models::{BindingRow, OccurrenceRow, ReferencesBucket, ScopeRow, SymbolInfo};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    let mut ctx = Ctx::new(file_path, source, symbols);
    let root = tree.root_node();
    let module_scope_id = ctx.push_scope(root, "module", None);
    ctx.emit_definitions(module_scope_id.clone(), symbols);
    ctx.walk(root, &module_scope_id);
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

    /// Pass 1: every Symbol → `definition` binding in the module scope.
    fn emit_definitions(&mut self, module_scope_id: String, symbols: &[SymbolInfo]) {
        for sym in symbols {
            // Parameters are bound at function scope during walk.
            if matches!(sym.kind, crate::models::SymbolKind::Parameter) {
                continue;
            }
            self.bucket.bindings.push(BindingRow {
                scope_id: module_scope_id.clone(),
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
            "function_definition" | "lambda" => {
                self.emit_function_params(node, &active_scope);
            }
            "import_statement" => {
                self.emit_import_statement(node, scope_id);
            }
            "import_from_statement" => {
                self.emit_import_from_statement(node, scope_id);
            }
            _ => {}
        }

        // Occurrence emission.
        if let Some(kind) = occurrence_kind_for(node, self.source) {
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

    /// Emit `parameter` bindings for every formal parameter of a
    /// `function_definition` or `lambda` node.
    fn emit_function_params(&mut self, node: Node, fn_scope: &str) {
        let params_field = match node.kind() {
            "function_definition" => "parameters",
            "lambda" => "parameters",
            _ => return,
        };
        let Some(params) = node.child_by_field_name(params_field) else {
            return;
        };
        let mut c = params.walk();
        for p in params.named_children(&mut c) {
            if let Some((name, start)) = parameter_name(p, self.source) {
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

    /// Handle `import foo`, `import foo.bar`, `import foo as bar`,
    /// `import a, b as c, ...`. Bind in the enclosing scope (`scope_id`),
    /// which is the nearest function or module scope (Python has no
    /// block scope).
    fn emit_import_statement(&mut self, node: Node, scope_id: &str) {
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            match child.kind() {
                "dotted_name" => {
                    // `import foo[.bar]*` — the binding is the head
                    // identifier of the dotted name (the local name
                    // introduced into scope).
                    if let Some(head) = head_identifier(child)
                        && let Ok(text) = head.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: scope_id.to_string(),
                            name: text.to_string(),
                            start_byte: head.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "import".to_string(),
                        });
                    }
                }
                "aliased_import" => {
                    // `import X as Y` — bind Y as import_alias.
                    if let Some(alias) = child.child_by_field_name("alias")
                        && let Ok(text) = alias.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: scope_id.to_string(),
                            name: text.to_string(),
                            start_byte: alias.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "import_alias".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle `from X import a`, `from X import a as b`,
    /// `from X import *`. The module-side name (`X`) is NOT a binding —
    /// the imported names are. `*` becomes a `wildcard_import` row.
    fn emit_import_from_statement(&mut self, node: Node, scope_id: &str) {
        // Walk past the `from <module>` prefix; collect the imported
        // names that come after the `import` keyword.
        let mut c = node.walk();
        let mut past_import_keyword = false;
        for child in node.children(&mut c) {
            let kind = child.kind();
            if !past_import_keyword {
                if kind == "import" {
                    past_import_keyword = true;
                }
                continue;
            }
            match kind {
                "dotted_name" => {
                    // `from X import name` — bind `name` (head identifier
                    // of the dotted name; in practice this is always a
                    // single identifier in `from ... import` lists).
                    if let Some(head) = head_identifier(child)
                        && let Ok(text) = head.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: scope_id.to_string(),
                            name: text.to_string(),
                            start_byte: head.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "import".to_string(),
                        });
                    }
                }
                "aliased_import" => {
                    // `from X import a as b` — bind `b` as import_alias.
                    if let Some(alias) = child.child_by_field_name("alias")
                        && let Ok(text) = alias.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: scope_id.to_string(),
                            name: text.to_string(),
                            start_byte: alias.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "import_alias".to_string(),
                        });
                    }
                }
                "wildcard_import" => {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope_id.to_string(),
                        name: "*".to_string(),
                        start_byte: child.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "wildcard_import".to_string(),
                    });
                }
                _ => {}
            }
        }
    }
}

/// Which scope (if any) does this node open?
fn scope_kind_for(node: Node) -> Option<&'static str> {
    match node.kind() {
        "function_definition" | "lambda" => Some("function"),
        "class_definition" => Some("class"),
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => Some("function"),
        _ => None,
    }
}

/// Extract the bound name from a `parameters` / `lambda_parameters`
/// child. Returns `(name, start_byte_of_name_node)`.
fn parameter_name(p: Node, source: &[u8]) -> Option<(String, u32)> {
    match p.kind() {
        "identifier" => {
            let text = p.utf8_text(source).ok()?;
            Some((text.to_string(), p.start_byte() as u32))
        }
        "typed_parameter" => {
            // First identifier child is the parameter name; the type
            // annotation lives in the `type` field. The grammar does
            // not give the name a field name, so we walk children.
            let mut c = p.walk();
            for child in p.named_children(&mut c) {
                if child.kind() == "identifier" {
                    let text = child.utf8_text(source).ok()?;
                    return Some((text.to_string(), child.start_byte() as u32));
                }
                if child.kind() == "list_splat_pattern"
                    || child.kind() == "dictionary_splat_pattern"
                {
                    return splat_name(child, source);
                }
            }
            None
        }
        "default_parameter" | "typed_default_parameter" => {
            let name_node = p.child_by_field_name("name")?;
            let text = name_node.utf8_text(source).ok()?;
            Some((text.to_string(), name_node.start_byte() as u32))
        }
        "list_splat_pattern" | "dictionary_splat_pattern" => splat_name(p, source),
        "tuple_pattern" => {
            // For `def f((a, b)):` the first identifier in the tuple
            // is the param name we can surface; richer destructuring
            // is out of scope for the extractor here.
            let mut c = p.walk();
            for child in p.named_children(&mut c) {
                if child.kind() == "identifier" {
                    let text = child.utf8_text(source).ok()?;
                    return Some((text.to_string(), child.start_byte() as u32));
                }
            }
            None
        }
        _ => None,
    }
}

fn splat_name(p: Node, source: &[u8]) -> Option<(String, u32)> {
    let mut c = p.walk();
    for child in p.named_children(&mut c) {
        if child.kind() == "identifier" {
            let text = child.utf8_text(source).ok()?;
            return Some((text.to_string(), child.start_byte() as u32));
        }
    }
    None
}

/// Return the head identifier of a `dotted_name` node, or the node
/// itself if it is already an identifier.
fn head_identifier(node: Node) -> Option<Node> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        if child.kind() == "identifier" {
            return Some(child);
        }
    }
    None
}

/// Classify the occurrence_kind of an identifier-shaped node based on
/// its parent context. Returns `None` for nodes that are NOT
/// occurrences (declarations, attribute tails, parameter names, etc.).
fn occurrence_kind_for(node: Node, _source: &[u8]) -> Option<&'static str> {
    if node.kind() != "identifier" {
        return None;
    }
    let Some(parent) = node.parent() else {
        return None;
    };
    let pkind = parent.kind();

    // Attribute tails: `obj.attr` — only the head `obj` produces a
    // read; `attr` (the `attribute` field of `attribute`) is suppressed.
    if pkind == "attribute"
        && parent.child_by_field_name("attribute").map(|n| n.id()) == Some(node.id())
    {
        return None;
    }

    // Keyword-argument keys: `f(name=value)` — the `name` side is a
    // local parameter name in the callee, not an occurrence of any
    // binding in the current scope.
    if pkind == "keyword_argument"
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        return None;
    }

    // Declaration NAMES (the identifier that IS the name field of a
    // `function_definition` / `class_definition`). These are bindings,
    // not occurrences.
    if (pkind == "function_definition" || pkind == "class_definition")
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        return None;
    }

    // Parameter-name positions inside `parameters` / `lambda_parameters`.
    // The walk emits `parameter` bindings; we suppress the identifier
    // occurrence so it doesn't double-up as a `read`.
    if is_inside_parameter_header(node) {
        return None;
    }

    // Import statements → import_use. The local-alias name is captured
    // by the `import_alias` binding, so suppress that one occurrence.
    if is_inside_import(parent) {
        // Inside an `aliased_import`, the `alias` field is the local
        // name — not an import_use.
        if pkind == "aliased_import"
            && parent.child_by_field_name("alias").map(|n| n.id()) == Some(node.id())
        {
            return None;
        }
        return Some("import_use");
    }

    // Type-annotation positions → type_use.
    if is_in_type_position(node) {
        return Some("type_use");
    }

    // Call: bare identifier in `function` field of a `call` node.
    if pkind == "call"
        && parent.child_by_field_name("function").map(|n| n.id()) == Some(node.id())
    {
        return Some("call");
    }

    // Assignment LHS — simple identifier in `left` field of `assignment`
    // or `augmented_assignment`. Tuple/list destructuring still walks
    // down to identifier nodes whose parent is `pattern_list` /
    // `tuple_pattern` / `list_pattern` — also treat those as writes.
    if (pkind == "assignment" || pkind == "augmented_assignment")
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }
    if matches!(pkind, "pattern_list" | "tuple_pattern" | "list_pattern")
        && is_assignment_target(parent)
    {
        return Some("write");
    }

    // `for x in iter:` — `x` in the `left` field is a write.
    if pkind == "for_statement"
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }

    // `for x in iter` comprehension clause — `x` is a write.
    if pkind == "for_in_clause"
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }

    // `with expr as x:` — the `as_pattern` wraps the binding identifier
    // (`x`) with the value expression on the left.
    if pkind == "as_pattern_target" {
        return Some("write");
    }

    // Walrus `(x := expr)` — assignment_expression's `name` field is the
    // written name; the right side is a normal read.
    if pkind == "named_expression"
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }

    // `global x` / `nonlocal x` — emit a write at the declaration site.
    // The accompanying `definition` binding (with null symbol_id) is
    // not emitted by this extractor; it's a follow-up. The write
    // occurrence at minimum marks the name's location.
    if pkind == "global_statement" || pkind == "nonlocal_statement" {
        return Some("write");
    }

    Some("read")
}

/// Walk up to check whether `node` sits inside a `parameters` or
/// `lambda_parameters` header — meaning the identifier is part of the
/// parameter declaration, not the function body.
fn is_inside_parameter_header(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "parameters" | "lambda_parameters" => return true,
            // Stop the search once we cross into a body / value
            // expression — default values inside `default_parameter`
            // are NOT parameter-name positions even though they are
            // textually inside `parameters`. Tree-sitter places the
            // default expression under a `value` field; we let it pass
            // by checking that the identifier's immediate ancestor chain
            // does NOT include a `value` field traversal. Simpler:
            // if the identifier's parent is `default_parameter` and the
            // identifier is the `value` field, return false.
            "default_parameter" | "typed_default_parameter" => {
                if p.child_by_field_name("value").map(|n| n.id()) == Some(node.id()) {
                    return false;
                }
                // Otherwise this identifier is the parameter name —
                // continue walking up (we expect `parameters` next).
            }
            "function_definition" | "lambda" => return false,
            _ => {}
        }
        cur = p.parent();
    }
    false
}

/// True if `parent` is on the `left` side of an `assignment` or
/// `augmented_assignment` (used to classify destructuring targets).
fn is_assignment_target(parent: Node) -> bool {
    let Some(gp) = parent.parent() else {
        return false;
    };
    match gp.kind() {
        "assignment" | "augmented_assignment" => {
            gp.child_by_field_name("left").map(|n| n.id()) == Some(parent.id())
        }
        "for_statement" | "for_in_clause" => {
            gp.child_by_field_name("left").map(|n| n.id()) == Some(parent.id())
        }
        _ => false,
    }
}

/// True if `parent` is part of an import statement node chain.
fn is_inside_import(parent: Node) -> bool {
    matches!(
        parent.kind(),
        "import_statement" | "import_from_statement" | "aliased_import" | "dotted_name"
    ) && {
        // For dotted_name, only treat as import context if IT sits
        // inside an import statement.
        if parent.kind() == "dotted_name" {
            let mut cur = parent.parent();
            while let Some(p) = cur {
                match p.kind() {
                    "import_statement" | "import_from_statement" | "aliased_import" => return true,
                    "function_definition" | "lambda" | "class_definition" | "module" => {
                        return false;
                    }
                    _ => cur = p.parent(),
                }
            }
            false
        } else {
            true
        }
    }
}

/// True if `node` sits in a type-annotation position (parameter type,
/// return type, annotated assignment, generic argument).
fn is_in_type_position(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "type" => return true,
            // Parameter / return / variable type annotations all wrap
            // the type expression in a `type` node, so the loop above
            // catches them. Bail at the nearest containing statement.
            "function_definition"
            | "lambda"
            | "class_definition"
            | "assignment"
            | "expression_statement"
            | "module"
            | "block" => return false,
            _ => cur = p.parent(),
        }
    }
    false
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> ReferencesBucket {
        let mut parser = create_parser(Language::Python).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(Language::Python).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, Language::Python);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn file_scope_emitted() {
        let b = run("def f():\n    pass\n", "test.py");
        let module_scope = b.scopes.iter().find(|s| s.parent_id.is_none());
        assert!(module_scope.is_some(), "module scope must exist");
        assert_eq!(module_scope.unwrap().kind, "module");
    }

    #[test]
    fn function_scope_emitted() {
        let b = run("def f():\n    pass\n", "test.py");
        assert!(b.scopes.iter().any(|s| s.kind == "function"));
    }

    #[test]
    fn definition_binding_emitted() {
        let b = run("def hello():\n    pass\n", "test.py");
        let d = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "definition" && x.name == "hello");
        assert!(d.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn parameter_binding_emitted() {
        let b = run("def f(x, y):\n    pass\n", "test.py");
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
    fn parameter_binding_with_default_and_splat() {
        let b = run(
            "def f(a, b=1, c: int = 2, *args, **kwargs):\n    pass\n",
            "test.py",
        );
        let names: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .map(|x| x.name.as_str())
            .collect();
        for expected in ["a", "b", "c", "args", "kwargs"] {
            assert!(
                names.contains(&expected),
                "expected `{expected}` param, got {names:?}"
            );
        }
    }

    #[test]
    fn import_binding_emitted() {
        let b = run("import os\n", "test.py");
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "os");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn from_import_binding_emitted() {
        let b = run("from os import path\n", "test.py");
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "path");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn import_alias_binding_emitted() {
        let b = run("import numpy as np\n", "test.py");
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "np");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn from_import_alias_binding_emitted() {
        let b = run("from foo import bar as baz\n", "test.py");
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "baz");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn wildcard_import_binding_emitted() {
        let b = run("from foo import *\n", "test.py");
        let w = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "wildcard_import" && x.name == "*");
        assert!(w.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn call_occurrence_emitted() {
        let b = run("def f():\n    g()\ndef g():\n    pass\n", "test.py");
        let c = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "call" && o.name == "g");
        assert!(c.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn write_occurrence_emitted() {
        let b = run("def f():\n    x = 1\n    x = 2\n", "test.py");
        let w_count = b
            .occurrences
            .iter()
            .filter(|o| o.occurrence_kind == "write" && o.name == "x")
            .count();
        assert!(w_count >= 1, "got: {:?}", b.occurrences);
    }

    #[test]
    fn class_scope_emitted() {
        let b = run("class Foo:\n    def bar(self):\n        pass\n", "test.py");
        assert!(b.scopes.iter().any(|s| s.kind == "class"));
    }

    #[test]
    fn comprehension_scope_emitted() {
        let b = run("def f():\n    xs = [d for d in data]\n", "test.py");
        // Two function scopes: one for `f`, one for the comprehension.
        let fn_scopes = b.scopes.iter().filter(|s| s.kind == "function").count();
        assert!(
            fn_scopes >= 2,
            "expected at least 2 function scopes (f + comprehension), got {fn_scopes}"
        );
    }
}
