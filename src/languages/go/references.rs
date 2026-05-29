//! Issue #16 Go `occurrence` / `scope` / `binding` fact emitter per
//! ADR-0005 and `docs/references-go.md`. The Cozoscript resolver
//! materialises `references` rows from these facts.
//!
//! Scope model:
//! - File root → `file` scope (`parent_id = null`). Imports bind here.
//!   The synthetic package/module scope described in the contract is
//!   not materialised by this extractor (cross-file aggregation happens
//!   at workspace level); top-level symbol definitions land in the file
//!   scope.
//! - `function_declaration` / `method_declaration` / `func_literal` →
//!   `function` scope. Parameters (including receiver and named returns)
//!   bind here.
//! - `block` → `block` scope. Covers function bodies and any nested
//!   `{ ... }` brace block (if/for/switch bodies parse as `block` in
//!   tree-sitter-go).
//!
//! Binding emission:
//! - Every `SymbolInfo` (except `Parameter`) → `definition` binding at
//!   the file scope. Parameter symbols bind at their function scope via
//!   the walk pass instead, so they don't double-count.
//! - `func`/`method` parameters, receivers, variadic params, and named
//!   return values → `parameter` bindings in the enclosing function
//!   scope. `symbol_id = None` (parameter binding rows don't carry a
//!   resolved symbol id).
//! - `short_var_declaration` (`x := …`) and `var_spec` (`var x = …`) →
//!   `definition` binding in the innermost enclosing scope (function or
//!   block).
//! - `import_declaration` → one `import` binding per spec at file scope,
//!   `name = <last path segment>`. An alias (`import b "x/y"`) becomes
//!   `import_alias` with `name = <alias>`. A dot-import
//!   (`import . "x/y"`) becomes `wildcard_import` with `name = "*"`.
//!   Blank `_` import emits no binding.
//!
//! Occurrence emission is intentionally a Level-3 narrowing matching the
//! contract: defining identifiers are skipped, blank `_` skipped, and
//! selector right-hand sides are not emitted in value position.

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

    /// Pass 1: every non-parameter Symbol → `definition` binding in file
    /// scope. Parameter symbols are bound at their function scope by the
    /// walk pass (see `emit_function_params`), so we skip them here to
    /// avoid double-counting.
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
            "function_declaration" | "method_declaration" | "func_literal" => {
                self.emit_function_params(node, &active_scope);
            }
            "import_declaration" => {
                self.emit_import_declaration(node, scope_id);
            }
            "short_var_declaration" => {
                self.emit_short_var(node, &active_scope);
            }
            "var_spec" => {
                self.emit_var_spec(node, &active_scope);
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
        if name.is_empty() || name == "_" {
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
    /// `function_declaration` / `method_declaration` / `func_literal`,
    /// including the method receiver and named return values.
    fn emit_function_params(&mut self, node: Node, fn_scope: &str) {
        // Receiver (method only).
        if node.kind() == "method_declaration"
            && let Some(recv) = node.child_by_field_name("receiver")
        {
            self.emit_parameter_list(recv, fn_scope);
        }
        // Regular parameters.
        if let Some(params) = node.child_by_field_name("parameters") {
            self.emit_parameter_list(params, fn_scope);
        }
        // Named return values: tree-sitter exposes them via the `result`
        // field. When the result is a `parameter_list` (named returns),
        // walk it like parameters. Bare types (no names) parse as a
        // single `type_identifier` or similar and produce no parameter
        // bindings.
        if let Some(result) = node.child_by_field_name("result")
            && result.kind() == "parameter_list"
        {
            self.emit_parameter_list(result, fn_scope);
        }
    }

    fn emit_parameter_list(&mut self, list: Node, fn_scope: &str) {
        let mut c = list.walk();
        for child in list.named_children(&mut c) {
            match child.kind() {
                "parameter_declaration" | "variadic_parameter_declaration" => {
                    // A single `parameter_declaration` can declare
                    // multiple names sharing one type (`a, b int`).
                    // Iterate every `identifier` child.
                    let mut nc = child.walk();
                    for n in child.named_children(&mut nc) {
                        if n.kind() == "identifier"
                            && let Ok(text) = n.utf8_text(self.source)
                            && !text.is_empty()
                            && text != "_"
                        {
                            self.bucket.bindings.push(BindingRow {
                                scope_id: fn_scope.to_string(),
                                name: text.to_string(),
                                start_byte: n.start_byte() as u32,
                                symbol_id: None,
                                binding_kind: "parameter".to_string(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Emit one binding per `import_spec` inside an `import_declaration`.
    /// Handles bare path, alias, dot-import, and blank-import forms.
    fn emit_import_declaration(&mut self, node: Node, scope_id: &str) {
        // `import_declaration` wraps either a single `import_spec` or an
        // `import_spec_list`. Recurse into both forms.
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            match child.kind() {
                "import_spec" => self.emit_import_spec(child, scope_id),
                "import_spec_list" => {
                    let mut sc = child.walk();
                    for spec in child.named_children(&mut sc) {
                        if spec.kind() == "import_spec" {
                            self.emit_import_spec(spec, scope_id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn emit_import_spec(&mut self, spec: Node, scope_id: &str) {
        let path_node = spec.child_by_field_name("path");
        let Some(path_node) = path_node else { return };
        let Ok(raw_path) = path_node.utf8_text(self.source) else {
            return;
        };
        let path = raw_path.trim_matches('"');
        if path.is_empty() {
            return;
        }

        // Optional alias / dot / blank in the `name` field.
        if let Some(name_node) = spec.child_by_field_name("name") {
            let Ok(name_text) = name_node.utf8_text(self.source) else {
                return;
            };
            match name_text {
                "_" => {
                    // Blank-import: side-effect-only, no binding row.
                }
                "." => {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope_id.to_string(),
                        name: "*".to_string(),
                        start_byte: name_node.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "wildcard_import".to_string(),
                    });
                }
                alias => {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope_id.to_string(),
                        name: alias.to_string(),
                        start_byte: name_node.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "import_alias".to_string(),
                    });
                }
            }
            return;
        }

        // Bare import — bind the last segment.
        let leaf = path.rsplit('/').next().unwrap_or(path);
        if leaf.is_empty() {
            return;
        }
        self.bucket.bindings.push(BindingRow {
            scope_id: scope_id.to_string(),
            name: leaf.to_string(),
            start_byte: path_node.start_byte() as u32,
            symbol_id: None,
            binding_kind: "import".to_string(),
        });
    }

    /// `x, y := …` — emit `definition` bindings for each LHS identifier
    /// in the innermost enclosing scope. The contract distinguishes
    /// definition (new name) from write (already-bound name); the
    /// extractor cannot reliably tell without scope tracking, so it
    /// emits `definition` for every LHS and lets the resolver pick the
    /// innermost matching binding.
    fn emit_short_var(&mut self, node: Node, scope: &str) {
        let Some(lhs) = node.child_by_field_name("left") else {
            return;
        };
        // `left` is an `expression_list` of identifiers.
        let mut c = lhs.walk();
        for child in lhs.named_children(&mut c) {
            if child.kind() == "identifier"
                && let Ok(text) = child.utf8_text(self.source)
                && !text.is_empty()
                && text != "_"
            {
                self.bucket.bindings.push(BindingRow {
                    scope_id: scope.to_string(),
                    name: text.to_string(),
                    start_byte: child.start_byte() as u32,
                    symbol_id: None,
                    binding_kind: "definition".to_string(),
                });
            }
        }
    }

    /// `var x [T] [= …]` — each `var_spec` can declare multiple names
    /// (`var a, b int`). The spec's `name` field is the first identifier
    /// but additional names are also `identifier` children of the spec.
    fn emit_var_spec(&mut self, node: Node, scope: &str) {
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.kind() == "identifier"
                && let Ok(text) = child.utf8_text(self.source)
                && !text.is_empty()
                && text != "_"
            {
                self.bucket.bindings.push(BindingRow {
                    scope_id: scope.to_string(),
                    name: text.to_string(),
                    start_byte: child.start_byte() as u32,
                    symbol_id: None,
                    binding_kind: "definition".to_string(),
                });
            }
        }
    }
}

/// Which scope (if any) does this node open?
fn scope_kind_for(node: Node) -> Option<&'static str> {
    match node.kind() {
        "function_declaration" | "method_declaration" | "func_literal" => Some("function"),
        // Owning construct verbatim (for_statement, if_statement, …)
        // instead of generic "block"; bare blocks report their parent.
        "block" => node.parent().map(|p| p.kind()),
        _ => None,
    }
}

/// Classify the occurrence_kind of an identifier-shaped node based on
/// its parent context. Returns `None` for nodes that are NOT occurrences
/// (declarations, selector right-hand sides in value position, etc.).
fn occurrence_kind_for(node: Node) -> Option<&'static str> {
    let kind = node.kind();
    if !matches!(kind, "identifier" | "type_identifier" | "field_identifier") {
        return None;
    }
    let parent = node.parent()?;
    let pk = parent.kind();

    // Defining-identifier positions — these names are bindings, not
    // occurrences.
    match pk {
        "function_declaration"
        | "method_declaration"
        | "type_spec"
        | "var_spec"
        | "const_spec"
        | "parameter_declaration"
        | "variadic_parameter_declaration"
        | "field_declaration"
        | "type_parameter_declaration"
        | "import_spec"
            if parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id()) =>
        {
            return None;
        }
        _ => {}
    }
    // LHS of `:=` — the names are definitions handled by `emit_short_var`.
    if pk == "expression_list"
        && let Some(gp) = parent.parent()
        && gp.kind() == "short_var_declaration"
        && gp.child_by_field_name("left").map(|n| n.id()) == Some(parent.id())
    {
        return None;
    }

    // Selector right-hand side in value position is NOT emitted (field
    // policy). `pkg.Name` and `obj.Method` emit only the LHS read.
    if pk == "selector_expression"
        && parent.child_by_field_name("field").map(|n| n.id()) == Some(node.id())
    {
        return None;
    }

    // type_identifier always denotes a type use.
    if kind == "type_identifier" {
        return Some("type_use");
    }

    // Call: identifier in `function` field of a `call_expression`. The
    // qualified form (`pkg.Func()`) places a `selector_expression` in
    // that slot; this branch fires only for bare-identifier callees.
    if pk == "call_expression"
        && parent.child_by_field_name("function").map(|n| n.id()) == Some(node.id())
    {
        return Some("call");
    }

    // Assignment LHS → write. Go's `assignment_statement` puts the LHS
    // in `left` (an expression_list). A bare identifier nested under
    // that list is a write.
    if pk == "expression_list"
        && let Some(gp) = parent.parent()
        && gp.kind() == "assignment_statement"
        && gp.child_by_field_name("left").map(|n| n.id()) == Some(parent.id())
    {
        // `x.Field = v` puts a selector_expression on the left, not a
        // bare identifier, so this only fires for `x = v` / `x, y = …`.
        return Some("write");
    }
    // `x++` / `x--`.
    if pk == "inc_statement" || pk == "dec_statement" {
        return Some("write");
    }

    Some("read")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> ReferencesBucket {
        let mut parser = create_parser(Language::Go).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(Language::Go).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, Language::Go);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn file_scope_emitted() {
        let b = run("package main\nfunc main() {}\n", "main.go");
        let file_scope = b.scopes.iter().find(|s| s.parent_id.is_none());
        assert!(file_scope.is_some(), "file scope must exist");
        assert_eq!(file_scope.unwrap().kind, "file");
    }

    #[test]
    fn function_scope_emitted() {
        let b = run("package main\nfunc main() {}\n", "main.go");
        assert!(b.scopes.iter().any(|s| s.kind == "function"));
    }

    #[test]
    fn parameter_binding_emitted() {
        let b = run("package main\nfunc f(x int, y string) {}\n", "main.go");
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
    fn method_receiver_parameter_binding() {
        let b = run(
            "package main\ntype Foo struct{}\nfunc (r *Foo) Bar(arg int) {}\n",
            "main.go",
        );
        let params: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .map(|x| x.name.as_str())
            .collect();
        assert!(
            params.contains(&"r"),
            "receiver `r` must be a parameter binding, got {params:?}"
        );
        assert!(
            params.contains(&"arg"),
            "method arg `arg` must be a parameter binding, got {params:?}"
        );
    }

    #[test]
    fn variadic_parameter_binding() {
        let b = run("package main\nfunc f(args ...int) {}\n", "main.go");
        let p = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "parameter" && x.name == "args");
        assert!(p.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn import_binding_emitted() {
        let b = run("package main\nimport \"net/http\"\n", "main.go");
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "http");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn grouped_import_binding_emitted() {
        let b = run(
            "package main\nimport (\n\t\"fmt\"\n\t\"net/http\"\n)\n",
            "main.go",
        );
        let names: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "import")
            .map(|x| x.name.as_str())
            .collect();
        assert!(names.contains(&"fmt"), "got: {:?}", b.bindings);
        assert!(names.contains(&"http"), "got: {:?}", b.bindings);
    }

    #[test]
    fn import_alias_binding_emitted() {
        let b = run(
            "package main\nimport log \"github.com/example/pkg/logger\"\n",
            "main.go",
        );
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "log");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn wildcard_import_binding_emitted() {
        let b = run("package main\nimport . \"fmt\"\n", "main.go");
        let w = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "wildcard_import" && x.name == "*");
        assert!(w.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn blank_import_emits_no_binding() {
        let b = run("package main\nimport _ \"net/http/pprof\"\n", "main.go");
        let any_import = b.bindings.iter().any(|x| {
            matches!(
                x.binding_kind.as_str(),
                "import" | "import_alias" | "wildcard_import"
            )
        });
        assert!(
            !any_import,
            "blank import must emit no binding, got: {:?}",
            b.bindings
        );
    }

    #[test]
    fn short_var_definition_binding() {
        let b = run("package main\nfunc f() { x := 1; _ = x }\n", "main.go");
        let d = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "definition" && x.name == "x");
        assert!(d.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn var_spec_definition_binding() {
        let b = run(
            "package main\nfunc f() { var foo int = 1; _ = foo }\n",
            "main.go",
        );
        let d = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "definition" && x.name == "foo");
        assert!(d.is_some(), "got: {:?}", b.bindings);
    }
}
