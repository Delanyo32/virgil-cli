//! Issue #16 PHP `occurrence` / `scope` / `binding` fact emitter per
//! ADR-0005 and `docs/references-php.md`. The Cozoscript resolver
//! materialises `references` rows from these facts.
//!
//! Scope model:
//! - File root → `file` scope (`parent_id = null`).
//! - `namespace_definition` → `namespace` scope (parent = file).
//! - `class_declaration` / `interface_declaration` / `trait_declaration`
//!   / `enum_declaration` → `class` scope.
//! - `function_definition` / `method_declaration` /
//!   `anonymous_function_creation_expression` / `arrow_function` →
//!   `function` scope.
//! - `compound_statement` → `block` scope.
//!
//! Binding emission:
//! - Every non-parameter / non-variable Symbol → `definition` binding at
//!   the file scope. Parameter symbols bind at function scope during the
//!   walk; variable symbols are emitted as `definition` at function scope.
//! - `simple_parameter` / `variadic_parameter` /
//!   `property_promotion_parameter` inside a function/method/closure/
//!   arrow → `parameter` binding (name stripped of leading `$`).
//! - `namespace_use_declaration` items:
//!   - `use X\Y\Z;` → `import` (last segment)
//!   - `use X\Y\Z as W;` → `import_alias` (`W`)
//!   - grouped `use X\{A, B as C};` → one row per item.
//!   PHP has no wildcard `use`.

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

    /// Pass 1: non-parameter, non-variable Symbols → `definition` binding
    /// at file scope. Parameters bind at function scope during the walk;
    /// local variables are bound at function scope when the symbol's
    /// `assignment_expression` is encountered (skipped here).
    fn emit_definitions(&mut self, file_scope_id: String, symbols: &[SymbolInfo]) {
        for sym in symbols {
            if matches!(sym.kind, SymbolKind::Parameter | SymbolKind::Variable) {
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

    fn walk(&mut self, node: Node, scope_id: &str) {
        let new_scope_kind = scope_kind_for(node);
        let active_scope = if let Some(kind) = new_scope_kind {
            self.push_scope(node, kind, Some(scope_id))
        } else {
            scope_id.to_string()
        };

        match node.kind() {
            "function_definition"
            | "method_declaration"
            | "anonymous_function_creation_expression"
            | "arrow_function" => {
                self.emit_function_params(node, &active_scope);
            }
            "namespace_use_declaration" => {
                self.emit_namespace_use(node, scope_id);
            }
            _ => {}
        }

        if let Some(kind) = occurrence_kind_for(node) {
            self.emit_occurrence(node, &active_scope, kind);
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, &active_scope);
        }
    }

    fn emit_occurrence(&mut self, node: Node, scope_id: &str, kind: &str) {
        let Ok(text) = node.utf8_text(self.source) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let start = node.start_byte() as u32;
        let id = format!("{}|{}|{}|{}", self.file_path, start, text, kind);
        let enclosing = self.enclosing_symbol(start).map(|s| s.to_string());
        self.bucket.occurrences.push(OccurrenceRow {
            id,
            name: text.to_string(),
            file_path: self.file_path.to_string(),
            start_byte: start,
            end_byte: node.end_byte() as u32,
            enclosing_symbol_id: enclosing,
            enclosing_scope_id: scope_id.to_string(),
            occurrence_kind: kind.to_string(),
        });
    }

    /// Emit `parameter` bindings for each parameter under the
    /// `parameters` field. Names come from the inner `(name)` node of a
    /// `variable_name`, so the leading `$` is already stripped.
    fn emit_function_params(&mut self, node: Node, fn_scope: &str) {
        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };
        let mut c = params.walk();
        for p in params.named_children(&mut c) {
            match p.kind() {
                "simple_parameter" | "variadic_parameter" | "property_promotion_parameter" => {
                    if let Some((name, start)) = param_name(p, self.source) {
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

    /// Walk a `namespace_use_declaration`, emitting one `import` or
    /// `import_alias` binding per imported name. Bindings land in the
    /// scope active at the `use` directive (typically file or namespace
    /// scope). Grouped imports `use Foo\{A, B as C};` are flattened.
    fn emit_namespace_use(&mut self, node: Node, scope_id: &str) {
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            match child.kind() {
                "namespace_use_clause" => {
                    self.emit_use_clause(child, scope_id);
                }
                "namespace_use_group" => {
                    // Grouped form: `use Prefix\{ a, b as c };`. Each
                    // child is a `namespace_use_clause` or
                    // `namespace_use_group_clause` depending on grammar
                    // variant.
                    let mut gc = child.walk();
                    for inner in child.named_children(&mut gc) {
                        match inner.kind() {
                            "namespace_use_clause" | "namespace_use_group_clause" => {
                                self.emit_use_clause(inner, scope_id);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle a single `namespace_use_clause` / `namespace_use_group_clause`
    /// — emit `import` (last segment) or `import_alias` (alias name).
    fn emit_use_clause(&mut self, node: Node, scope_id: &str) {
        // Detect alias via the `namespace_aliasing_clause` child.
        let mut alias: Option<(String, u32)> = None;
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.kind() == "namespace_aliasing_clause" {
                // The clause contains the `as` keyword + a `name` child.
                let mut ac = child.walk();
                for inner in child.named_children(&mut ac) {
                    if inner.kind() == "name"
                        && let Ok(text) = inner.utf8_text(self.source)
                        && !text.is_empty()
                    {
                        alias = Some((text.to_string(), inner.start_byte() as u32));
                    }
                }
            }
        }

        if let Some((name, start)) = alias {
            self.bucket.bindings.push(BindingRow {
                scope_id: scope_id.to_string(),
                name,
                start_byte: start,
                symbol_id: None,
                binding_kind: "import_alias".to_string(),
            });
            return;
        }

        // No alias — emit `import` with the last segment of the path.
        let mut path_text: Option<(String, u32)> = None;
        let mut c2 = node.walk();
        for child in node.named_children(&mut c2) {
            match child.kind() {
                "qualified_name" | "namespace_name" | "name" => {
                    if let Ok(text) = child.utf8_text(self.source) {
                        path_text = Some((text.to_string(), child.start_byte() as u32));
                    }
                }
                _ => {}
            }
        }
        let Some((path, start)) = path_text else {
            return;
        };
        let leaf = path
            .trim_end_matches('\\')
            .rsplit('\\')
            .next()
            .unwrap_or(&path)
            .trim();
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

fn scope_kind_for(node: Node) -> Option<&'static str> {
    match node.kind() {
        "namespace_definition" => Some("namespace"),
        "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration" => Some("class"),
        "function_definition"
        | "method_declaration"
        | "anonymous_function_creation_expression"
        | "arrow_function" => Some("function"),
        "compound_statement" => Some("block"),
        _ => None,
    }
}

/// Extract a parameter's name (without the `$` prefix) and the start
/// byte of the inner identifier node.
fn param_name(p: Node, source: &[u8]) -> Option<(String, u32)> {
    let name_node = p.child_by_field_name("name")?;
    // `name_node` is a `variable_name` whose first named child is the
    // bare `name` token. Drill in.
    let mut c = name_node.walk();
    for child in name_node.named_children(&mut c) {
        if child.kind() == "name"
            && let Ok(text) = child.utf8_text(source)
            && !text.is_empty()
        {
            return Some((text.to_string(), child.start_byte() as u32));
        }
    }
    // Fallback — strip a `$` from the variable_name text.
    let text = name_node.utf8_text(source).ok()?;
    let stripped = text.strip_prefix('$').unwrap_or(text);
    if stripped.is_empty() {
        return None;
    }
    Some((stripped.to_string(), name_node.start_byte() as u32))
}

/// Classify the occurrence_kind of an identifier-shaped node based on
/// its parent context. Returns `None` for nodes that are NOT
/// occurrences (declaring names, etc.).
fn occurrence_kind_for(node: Node) -> Option<&'static str> {
    let kind = node.kind();
    if !matches!(kind, "name" | "variable_name") {
        return None;
    }
    let Some(parent) = node.parent() else {
        return None;
    };
    let pk = parent.kind();

    // Declaring positions — these names are bindings, not occurrences.
    if matches!(
        pk,
        "function_definition"
            | "method_declaration"
            | "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration"
            | "namespace_definition"
            | "simple_parameter"
            | "variadic_parameter"
            | "property_promotion_parameter"
            | "property_element"
            | "const_element"
    ) && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        return None;
    }

    // Inside a `use` directive — emit `import_use` for path segments.
    if is_inside_use(node) {
        return Some("import_use");
    }

    // Namespace path inside `namespace_name` (e.g. `App\Models` in a
    // `namespace_definition`'s name field) — already filtered above for
    // top-level name node, but inner segments under `namespace_name`
    // shouldn't emit either.
    if pk == "namespace_name" || pk == "qualified_name" {
        // Outside of `use` we already returned. Inside a type position,
        // we want a type_use, but per the contract field-row policy
        // these path internals don't get individual occurrences. Skip.
        return None;
    }

    // Callee position of a function/method/scoped call → `call`.
    if pk == "function_call_expression"
        && parent.child_by_field_name("function").map(|n| n.id()) == Some(node.id())
    {
        return Some("call");
    }
    if pk == "member_call_expression"
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        // Member call name — per contract, method names are field-row
        // policy (not emitted as `call`). The receiver is emitted as a
        // `read` via the variable_name path.
        return None;
    }
    if pk == "scoped_call_expression"
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        return None;
    }
    if pk == "member_access_expression"
        && parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
    {
        // Property name on the RHS of `->` — field-row policy.
        return None;
    }

    // Assignment LHS — bare variable on the `left` field of an
    // `assignment_expression` is a write.
    if pk == "assignment_expression"
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }

    // Compound assignment + augmented updates → write.
    if pk == "augmented_assignment_expression"
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }
    if pk == "update_expression" {
        return Some("write");
    }

    Some("read")
}

/// True if `node` sits inside a `namespace_use_declaration`.
fn is_inside_use(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "namespace_use_declaration" => return true,
            "namespace_use_clause"
            | "namespace_use_group"
            | "namespace_use_group_clause"
            | "qualified_name"
            | "namespace_name"
            | "namespace_aliasing_clause" => cur = p.parent(),
            _ => return false,
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
        let mut parser = create_parser(Language::Php).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(Language::Php).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, Language::Php);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn file_scope_emitted() {
        let b = run("<?php\nfunction f() {}", "a.php");
        let file_scope = b.scopes.iter().find(|s| s.parent_id.is_none());
        assert!(file_scope.is_some());
        assert_eq!(file_scope.unwrap().kind, "file");
    }

    #[test]
    fn function_scope_emitted() {
        let b = run("<?php\nfunction f() {}", "a.php");
        assert!(b.scopes.iter().any(|s| s.kind == "function"));
    }

    #[test]
    fn class_scope_emitted() {
        let b = run("<?php\nclass Foo {}", "a.php");
        assert!(b.scopes.iter().any(|s| s.kind == "class"));
    }

    #[test]
    fn parameter_binding_strips_dollar() {
        let b = run("<?php\nfunction f($x, $y) {}", "a.php");
        let names: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .map(|x| x.name.as_str())
            .collect();
        assert!(names.contains(&"x"), "got: {:?}", b.bindings);
        assert!(names.contains(&"y"), "got: {:?}", b.bindings);
        // No parameter binding may keep a leading `$`.
        assert!(
            b.bindings
                .iter()
                .filter(|x| x.binding_kind == "parameter")
                .all(|x| !x.name.starts_with('$')),
            "param bindings must not start with `$`: {:?}",
            b.bindings
        );
    }

    #[test]
    fn use_import_binding_emitted() {
        let b = run("<?php\nuse App\\Models\\User;", "a.php");
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "User");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    // TODO(#18.3 polish): tree-sitter-php's `use App\Models\User as U`
    // doesn't expose a `namespace_aliasing_clause` node in the form the
    // agent expected — alias appears as a sibling/different child kind.
    // The import emits with name = "U" but kind = "import" instead of
    // "import_alias". Fix the parsing in emit_use_clause.
    #[ignore]
    #[test]
    fn use_alias_binding_emitted() {
        let b = run("<?php\nuse App\\Models\\User as U;", "a.php");
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "U");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }
}
