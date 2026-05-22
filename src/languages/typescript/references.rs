//! Issue #16 TypeScript/JavaScript `occurrence` / `scope` / `binding`
//! fact emitter per ADR-0005. The Cozoscript resolver materialises
//! `references` rows from these facts (see `docs/references-typescript.md`).
//!
//! Scope model:
//! - `program` → `file` scope (always emitted).
//! - `internal_module` / `module` → `namespace` scope (TS-only AST nodes;
//!   in JS the grammar does not produce them).
//! - `class_declaration` / `class_expression` / `abstract_class_declaration`
//!   → `class` scope.
//! - `function_declaration` / `function_expression` / `arrow_function` /
//!   `method_definition` / `generator_function_declaration` /
//!   `generator_function` → `function` scope.
//! - `statement_block` → `block` scope, except when it sits as the body of
//!   a function-like (we keep the function's `function` scope instead of
//!   nesting an extra block underneath).
//! - `for_statement` / `for_in_statement` / `for_of_statement` →
//!   `block` scope (wraps the header + body).
//! - `catch_clause` → `block` scope.
//! - `switch_body` → `block` scope.
//!
//! Binding emission:
//! - Every Symbol becomes a `definition` binding in the file scope
//!   (top-level resolver entry point). Methods + class fields also bind
//!   in their enclosing class scope when applicable — emitted from the
//!   walk so the `scope_id` is the class scope, not file.
//! - Function / arrow / method parameters become `parameter` bindings
//!   in their function scope.
//! - `lexical_declaration` (`let` / `const`) → `definition` in the
//!   innermost block/function scope. `variable_declaration` (`var`) →
//!   `definition` in the enclosing function/file scope (we approximate
//!   with the closest function/file scope captured during walk).
//! - `import_statement` clauses emit `import`, `import_alias`, and the
//!   namespace alias case (`import * as ns`).
//! - `export_statement` with `source` and `export_clause` emits
//!   `import_alias` for the renamed binding. Bare `export * from`
//!   emits a `wildcard_import` row with `name = "*"`.
//!
//! Occurrence emission:
//! - `call_expression` callee identifier / `new_expression` constructor
//!   identifier → `call`.
//! - LHS of `assignment_expression` (simple identifier) → `write`.
//!   Compound `augmented_assignment_expression` LHS → `write` (no
//!   separate `read`, per ADR-0003). `update_expression` operand → `write`.
//! - Identifiers in a type position (under `type_annotation`,
//!   `type_arguments`, `extends_type_clause`, `implements_clause`,
//!   `type_alias_declaration`, `interface_body`, `as_expression`,
//!   `satisfies_expression`, `predicate_type`, `index_type_query` etc.)
//!   → `type_use`. JS files do not contain these nodes, so they emit
//!   none.
//! - Identifiers inside `import_statement` / `export_statement` (with
//!   a `source`) → `import_use`. Binding sites (the local name in an
//!   alias / namespace import) are skipped — they are recorded as
//!   bindings, not occurrences.
//! - Property names in `member_expression` (the `.foo` part) and
//!   object-property keys emit no occurrence (field-row policy).
//! - Default: any other identifier in value position → `read`.

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
    let file_scope_id = ctx.push_scope(root, "file", None);
    ctx.emit_definitions(file_scope_id.clone(), symbols);
    ctx.walk(root, &file_scope_id);
    ctx.finish()
}

struct Ctx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    bucket: ReferencesBucket,
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

    fn emit_definitions(&mut self, file_scope_id: String, symbols: &[SymbolInfo]) {
        for sym in symbols {
            // Skip parameters here — they are bound at function scope
            // during the walk so the `scope_id` points at the enclosing
            // function, not the file.
            if matches!(sym.kind, crate::models::SymbolKind::Parameter) {
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

        // statement_block that is the immediate body of a function-like
        // is NOT its own scope — the function scope already covers it.
        let active_scope = if let Some(kind) = new_scope_kind {
            if kind == "block" && is_function_body_block(node) {
                scope_id.to_string()
            } else {
                self.push_scope(node, kind, Some(scope_id))
            }
        } else {
            scope_id.to_string()
        };

        // Emit bindings unique to this node kind.
        match node.kind() {
            "function_declaration"
            | "function_expression"
            | "generator_function"
            | "generator_function_declaration"
            | "method_definition"
            | "arrow_function" => {
                self.emit_function_params(node, &active_scope);
            }
            "catch_clause" => {
                // catch (err) — bind `err` as parameter in this block scope.
                if let Some(param) = node.child_by_field_name("parameter") {
                    self.emit_pattern_bindings(param, &active_scope, "parameter");
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                self.emit_variable_declarators(node, &active_scope);
            }
            "import_statement" => {
                self.emit_import_bindings(node, scope_id);
            }
            "export_statement" => {
                self.emit_export_bindings(node, scope_id);
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

    fn emit_function_params(&mut self, node: Node, scope_id: &str) {
        // Arrow-function bare-identifier parameter: `x => x + 1`.
        if node.kind() == "arrow_function"
            && let Some(p) = node.child_by_field_name("parameter")
            && p.kind() == "identifier"
            && let Ok(name) = p.utf8_text(self.source)
        {
            self.bucket.bindings.push(BindingRow {
                scope_id: scope_id.to_string(),
                name: name.to_string(),
                start_byte: p.start_byte() as u32,
                symbol_id: None,
                binding_kind: "parameter".to_string(),
            });
        }

        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };
        let mut c = params.walk();
        for p in params.named_children(&mut c) {
            match p.kind() {
                "required_parameter" | "optional_parameter" => {
                    if let Some(pat) = p.child_by_field_name("pattern") {
                        self.emit_pattern_bindings(pat, scope_id, "parameter");
                    }
                }
                "identifier" => {
                    if let Ok(name) = p.utf8_text(self.source) {
                        self.bucket.bindings.push(BindingRow {
                            scope_id: scope_id.to_string(),
                            name: name.to_string(),
                            start_byte: p.start_byte() as u32,
                            symbol_id: None,
                            binding_kind: "parameter".to_string(),
                        });
                    }
                }
                "rest_pattern" | "assignment_pattern" | "object_pattern" | "array_pattern" => {
                    self.emit_pattern_bindings(p, scope_id, "parameter");
                }
                _ => {}
            }
        }
    }

    /// Recursively extract identifier bindings from a destructuring /
    /// rest / assignment pattern node and emit them under `kind`.
    fn emit_pattern_bindings(&mut self, node: Node, scope_id: &str, kind: &str) {
        match node.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                if let Ok(name) = node.utf8_text(self.source) {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope_id.to_string(),
                        name: name.to_string(),
                        start_byte: node.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: kind.to_string(),
                    });
                }
            }
            "rest_pattern" | "assignment_pattern" => {
                // rest: `...rest`; assignment: `x = 1`. Drill into the
                // first identifier-ish child.
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    self.emit_pattern_bindings(child, scope_id, kind);
                    // assignment_pattern: stop after the LHS so we don't
                    // descend into the RHS default-value expression.
                    if node.kind() == "assignment_pattern" {
                        break;
                    }
                }
            }
            "object_pattern" => {
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    match child.kind() {
                        // `{ y: z }` — `z` is the bound name (right side).
                        "pair_pattern" => {
                            if let Some(v) = child.child_by_field_name("value") {
                                self.emit_pattern_bindings(v, scope_id, kind);
                            }
                        }
                        // `{ x }` — shorthand.
                        "shorthand_property_identifier_pattern" | "identifier" => {
                            self.emit_pattern_bindings(child, scope_id, kind);
                        }
                        "rest_pattern" | "object_assignment_pattern" => {
                            self.emit_pattern_bindings(child, scope_id, kind);
                        }
                        _ => {}
                    }
                }
            }
            "array_pattern" => {
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    self.emit_pattern_bindings(child, scope_id, kind);
                }
            }
            "object_assignment_pattern" => {
                // `{ x = default }` — LHS only.
                if let Some(l) = node.child_by_field_name("left") {
                    self.emit_pattern_bindings(l, scope_id, kind);
                }
            }
            // Type-annotated patterns wrap an inner pattern.
            _ => {
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    if matches!(
                        child.kind(),
                        "identifier"
                            | "shorthand_property_identifier_pattern"
                            | "rest_pattern"
                            | "assignment_pattern"
                            | "object_pattern"
                            | "array_pattern"
                            | "object_assignment_pattern"
                    ) {
                        self.emit_pattern_bindings(child, scope_id, kind);
                    }
                }
            }
        }
    }

    fn emit_variable_declarators(&mut self, decl_node: Node, scope_id: &str) {
        let mut c = decl_node.walk();
        for child in decl_node.named_children(&mut c) {
            if child.kind() == "variable_declarator"
                && let Some(name_node) = child.child_by_field_name("name")
            {
                self.emit_pattern_bindings(name_node, scope_id, "definition");
            }
        }
    }

    fn emit_import_bindings(&mut self, node: Node, scope_id: &str) {
        // import { foo, bar as baz } from "./x";  → import / import_alias
        // import x from "./x";                    → import_alias (default)
        // import * as ns from "./x";              → import_alias (namespace)
        // import "./side-effect";                 → no binding
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() != "import_clause" {
                continue;
            }
            let mut cc = child.walk();
            for sub in child.children(&mut cc) {
                match sub.kind() {
                    "identifier" => {
                        // default import: `import x from ...`
                        if let Ok(name) = sub.utf8_text(self.source) {
                            self.bucket.bindings.push(BindingRow {
                                scope_id: scope_id.to_string(),
                                name: name.to_string(),
                                start_byte: sub.start_byte() as u32,
                                symbol_id: None,
                                binding_kind: "import_alias".to_string(),
                            });
                        }
                    }
                    "namespace_import" => {
                        // import * as ns from ...
                        let mut nc = sub.walk();
                        for nchild in sub.children(&mut nc) {
                            if nchild.kind() == "identifier"
                                && let Ok(name) = nchild.utf8_text(self.source)
                            {
                                self.bucket.bindings.push(BindingRow {
                                    scope_id: scope_id.to_string(),
                                    name: name.to_string(),
                                    start_byte: nchild.start_byte() as u32,
                                    symbol_id: None,
                                    binding_kind: "import_alias".to_string(),
                                });
                            }
                        }
                    }
                    "named_imports" => {
                        let mut nc = sub.walk();
                        for spec in sub.named_children(&mut nc) {
                            if spec.kind() != "import_specifier" {
                                continue;
                            }
                            self.emit_import_specifier(spec, scope_id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn emit_import_specifier(&mut self, spec: Node, scope_id: &str) {
        // import_specifier shape:
        //   name: <identifier|type_identifier>   (imported name)
        //   alias: <identifier|type_identifier>  (local name, optional)
        let imported = spec.child_by_field_name("name");
        let alias = spec.child_by_field_name("alias");
        if let Some(alias) = alias {
            if let Ok(local) = alias.utf8_text(self.source) {
                self.bucket.bindings.push(BindingRow {
                    scope_id: scope_id.to_string(),
                    name: local.to_string(),
                    start_byte: alias.start_byte() as u32,
                    symbol_id: None,
                    binding_kind: "import_alias".to_string(),
                });
            }
        } else if let Some(imp) = imported
            && let Ok(name) = imp.utf8_text(self.source)
        {
            self.bucket.bindings.push(BindingRow {
                scope_id: scope_id.to_string(),
                name: name.to_string(),
                start_byte: imp.start_byte() as u32,
                symbol_id: None,
                binding_kind: "import".to_string(),
            });
        }
    }

    fn emit_export_bindings(&mut self, node: Node, scope_id: &str) {
        // We only care about re-exports here: `export * from`,
        // `export { foo } from`, `export { foo as bar } from`.
        // `export <decl>` is handled by the underlying definition.
        let has_source = node.child_by_field_name("source").is_some();
        if !has_source {
            return;
        }
        let mut c = node.walk();
        for child in node.children(&mut c) {
            match child.kind() {
                // `export * from "./x"`.
                "*" => {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope_id.to_string(),
                        name: "*".to_string(),
                        start_byte: child.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "wildcard_import".to_string(),
                    });
                }
                "export_clause" => {
                    // Renamed re-exports become an `import_alias` row in
                    // the re-exporting file's scope, mirroring the contract.
                    let mut cc = child.walk();
                    for spec in child.named_children(&mut cc) {
                        if spec.kind() == "export_specifier" {
                            self.emit_export_specifier(spec, scope_id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn emit_export_specifier(&mut self, spec: Node, scope_id: &str) {
        let alias = spec.child_by_field_name("alias");
        if let Some(alias) = alias
            && let Ok(local) = alias.utf8_text(self.source)
        {
            self.bucket.bindings.push(BindingRow {
                scope_id: scope_id.to_string(),
                name: local.to_string(),
                start_byte: alias.start_byte() as u32,
                symbol_id: None,
                binding_kind: "import_alias".to_string(),
            });
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
}

fn scope_kind_for(node: Node) -> Option<&'static str> {
    match node.kind() {
        "program" => Some("file"),
        "internal_module" | "module" => Some("namespace"),
        "class_declaration" | "class_expression" | "abstract_class_declaration" => Some("class"),
        "function_declaration"
        | "function_expression"
        | "arrow_function"
        | "method_definition"
        | "generator_function"
        | "generator_function_declaration" => Some("function"),
        "statement_block" | "for_statement" | "for_in_statement" | "for_of_statement"
        | "catch_clause" | "switch_body" => Some("block"),
        _ => None,
    }
}

/// True if a `statement_block` sits directly as the body of a function /
/// method / arrow / generator. In that case we do NOT push a new `block`
/// scope — the enclosing `function` scope already covers it.
fn is_function_body_block(node: Node) -> bool {
    if node.kind() != "statement_block" {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    matches!(
        parent.kind(),
        "function_declaration"
            | "function_expression"
            | "arrow_function"
            | "method_definition"
            | "generator_function"
            | "generator_function_declaration"
    ) && parent.child_by_field_name("body").map(|b| b.id()) == Some(node.id())
}

/// True when `node` is the `name` field of `parent` — used to skip
/// identifiers in declaring position so they do not double as
/// occurrences.
fn is_name_field(node: Node, parent: Node) -> bool {
    parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id())
}

/// True if any ancestor up to a sensible boundary is one of the type
/// position node kinds. Used to flag TS type-position identifiers.
fn ancestor_is_type_position(mut node: Node) -> bool {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "type_annotation"
            | "opting_type_annotation"
            | "omitting_type_annotation"
            | "type_arguments"
            | "type_parameters"
            | "type_parameter"
            | "type_alias_declaration"
            | "interface_body"
            | "interface_declaration"
            | "extends_type_clause"
            | "implements_clause"
            | "predicate_type"
            | "type_predicate"
            | "as_expression"
            | "satisfies_expression"
            | "type_assertion"
            | "index_type_query"
            | "type_query"
            | "generic_type"
            | "nested_type_identifier"
            | "union_type"
            | "intersection_type"
            | "array_type"
            | "tuple_type"
            | "function_type"
            | "constructor_type"
            | "object_type"
            | "lookup_type"
            | "literal_type"
            | "readonly_type"
            | "conditional_type"
            | "mapped_type_clause"
            | "infer_type"
            | "template_literal_type" => return true,
            // Stop ascending when we hit a value-position boundary.
            "function_declaration"
            | "function_expression"
            | "arrow_function"
            | "method_definition"
            | "class_body"
            | "statement_block"
            | "program" => return false,
            _ => {}
        }
        node = parent;
    }
    false
}

/// True if any ancestor is an `import_statement` or `export_statement`
/// (with a source). Identifiers under such nodes — that are not the
/// local binding site — are `import_use` occurrences.
fn ancestor_is_import_or_export(mut node: Node) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == "import_statement" {
            return true;
        }
        if parent.kind() == "export_statement" {
            return parent.child_by_field_name("source").is_some();
        }
        node = parent;
    }
    false
}

/// True when `node` is the local binding site within an import
/// (default name, namespace alias `ns`, or the alias in `foo as bar`).
fn is_import_binding_site(node: Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "import_clause" => true, // default-import identifier
        "namespace_import" => true,
        "import_specifier" | "export_specifier" => {
            // The alias is the binding site when present; without an
            // alias the `name` field is both the imported and local
            // name → emit `import_use` (so do NOT treat as binding site).
            if let Some(alias) = parent.child_by_field_name("alias") {
                alias.id() == node.id()
            } else {
                false
            }
        }
        _ => false,
    }
}

/// True when `node` is the property name of a `member_expression`
/// (`obj.foo` — `foo` is skipped) or a `subscript_expression`
/// computed key (`obj[x]` — `x` is a normal expression, NOT skipped
/// — that case returns false here).
fn is_property_name_position(node: Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "member_expression" => {
            parent.child_by_field_name("property").map(|p| p.id()) == Some(node.id())
        }
        // Object literal property key: `{ key: value }`.
        "pair" => parent.child_by_field_name("key").map(|p| p.id()) == Some(node.id()),
        // JSX attribute name.
        "jsx_attribute" => true,
        _ => false,
    }
}

/// Classify the occurrence_kind of an identifier-ish node based on its
/// context. Returns `None` for nodes that are NOT occurrences
/// (declarations, scope markers, binding sites, property names).
fn occurrence_kind_for(node: Node, _source: &[u8]) -> Option<&'static str> {
    let kind = node.kind();
    // Only identifier-shaped tokens are occurrence candidates.
    let is_id = matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "this"
            | "super"
    );
    if !is_id {
        return None;
    }
    let parent = node.parent()?;

    // Skip property-name positions (member_expression `.foo`, object
    // literal keys, JSX attributes).
    if is_property_name_position(node) {
        return None;
    }

    // Skip identifiers that ARE the declared name of a binding site.
    match parent.kind() {
        "function_declaration"
        | "function_expression"
        | "generator_function_declaration"
        | "generator_function"
        | "class_declaration"
        | "class_expression"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration"
        | "method_definition"
        | "public_field_definition"
        | "field_definition"
        | "method_signature"
        | "abstract_method_signature"
        | "internal_module"
        | "module"
        | "variable_declarator"
        | "required_parameter"
        | "optional_parameter"
        | "enum_assignment"
        | "property_signature"
            if is_name_field(node, parent) =>
        {
            return None;
        }
        // `shorthand_property_identifier` in an object literal: `{ foo }`
        // — emit as `read` per contract; do NOT skip.
        _ => {}
    }

    // Patterns: identifier inside a destructuring pattern is a binding
    // site, not an occurrence.
    let mut p = parent;
    loop {
        match p.kind() {
            "object_pattern"
            | "array_pattern"
            | "rest_pattern"
            | "assignment_pattern"
            | "object_assignment_pattern"
            | "pair_pattern" => return None,
            // shorthand_property_identifier_pattern is itself the binding name.
            _ => {}
        }
        if matches!(
            p.kind(),
            "variable_declarator"
                | "required_parameter"
                | "optional_parameter"
                | "formal_parameters"
                | "arrow_function"
                | "catch_clause"
        ) {
            break;
        }
        let Some(parent_of_p) = p.parent() else {
            break;
        };
        // Only ascend a couple of levels — patterns are local.
        if matches!(
            parent_of_p.kind(),
            "program" | "statement_block" | "class_body"
        ) {
            break;
        }
        p = parent_of_p;
    }

    // Import / export binding sites.
    if is_import_binding_site(node) {
        return None;
    }

    // Inside an `import_statement` / `export_statement` (with source) →
    // `import_use` (skip the local-binding sites handled above).
    if ancestor_is_import_or_export(node) {
        return Some("import_use");
    }

    // Type positions (TS only — JS files have no such ancestors).
    if ancestor_is_type_position(node) || kind == "type_identifier" {
        return Some("type_use");
    }

    // Now decide value-position classification.
    match parent.kind() {
        // Callee of a call.
        "call_expression" => {
            if parent.child_by_field_name("function").map(|n| n.id()) == Some(node.id()) {
                Some("call")
            } else {
                Some("read")
            }
        }
        // Constructor in `new Foo(args)`.
        "new_expression" => {
            if parent.child_by_field_name("constructor").map(|n| n.id()) == Some(node.id()) {
                Some("call")
            } else {
                Some("read")
            }
        }
        // LHS of `x = y` (simple identifier LHS only — `obj.x = y`
        // has parent `member_expression`, handled by property skip).
        "assignment_expression" => {
            if parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id()) {
                Some("write")
            } else {
                Some("read")
            }
        }
        // Compound `x += 1`, `x ||= y`, etc.
        "augmented_assignment_expression" => {
            if parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id()) {
                Some("write")
            } else {
                Some("read")
            }
        }
        // `x++`, `++x`, `x--`, `--x`.
        "update_expression" => {
            if parent.child_by_field_name("argument").map(|n| n.id()) == Some(node.id()) {
                Some("write")
            } else {
                Some("read")
            }
        }
        // Tag template: `` tag`...` `` — `tag` is a call.
        "template_string" => Some("read"),
        _ => Some("read"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str, lang: Language) -> ReferencesBucket {
        let mut parser = create_parser(lang).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(lang).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, lang);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    fn run_ts(src: &str) -> ReferencesBucket {
        run(src, "test.ts", Language::TypeScript)
    }

    fn run_js(src: &str) -> ReferencesBucket {
        run(src, "test.js", Language::JavaScript)
    }

    #[test]
    fn file_scope_emitted_ts() {
        let b = run_ts("function main() {}");
        assert!(
            b.scopes
                .iter()
                .any(|s| s.kind == "file" && s.parent_id.is_none())
        );
    }

    #[test]
    fn function_scope_emitted_ts() {
        let b = run_ts("function main() {}");
        assert!(b.scopes.iter().any(|s| s.kind == "function"));
    }

    #[test]
    fn definition_binding_emitted_ts() {
        let b = run_ts("function main() {}");
        assert!(
            b.bindings
                .iter()
                .any(|x| x.binding_kind == "definition" && x.name == "main")
        );
    }

    #[test]
    fn parameter_binding_emitted_ts() {
        let b = run_ts("function f(x: number) { return x; }");
        let p = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "parameter" && x.name == "x");
        assert!(p.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn arrow_bare_param_binding_emitted() {
        let b = run_ts("const f = y => y * 2;");
        let p = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "parameter" && x.name == "y");
        assert!(p.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn named_import_binding_emitted() {
        let b = run_ts(r#"import { foo } from "./x";"#);
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "foo");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn aliased_import_binding_emitted() {
        let b = run_ts(r#"import { foo as bar } from "./x";"#);
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "bar");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn namespace_import_binding_emitted() {
        let b = run_ts(r#"import * as ns from "./x";"#);
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "ns");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn default_import_binding_emitted() {
        let b = run_ts(r#"import React from "react";"#);
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "React");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn wildcard_reexport_binding_emitted() {
        let b = run_ts(r#"export * from "./x";"#);
        let w = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "wildcard_import");
        assert!(w.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn call_occurrence_emitted_ts() {
        let b = run_ts("function f() { g(); } function g() {}");
        let c = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "call" && o.name == "g");
        assert!(c.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn new_expression_emits_call() {
        let b = run_ts("function f() { return new Foo(); } class Foo {}");
        let c = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "call" && o.name == "Foo");
        assert!(c.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn write_occurrence_emitted_ts() {
        let b = run_ts("function f() { let x = 1; x = 2; }");
        let w = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "write" && o.name == "x");
        assert!(w.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn compound_assignment_emits_write() {
        let b = run_ts("function f() { let x = 1; x += 2; }");
        let w = b
            .occurrences
            .iter()
            .filter(|o| o.occurrence_kind == "write" && o.name == "x")
            .count();
        assert!(
            w >= 1,
            "expected at least one write for x, got: {:?}",
            b.occurrences
        );
    }

    #[test]
    fn update_expression_emits_write() {
        let b = run_ts("function f() { let i = 0; i++; }");
        let w = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "write" && o.name == "i");
        assert!(w.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn type_use_occurrence_emitted_ts() {
        let b = run_ts("function f(x: Foo) {} interface Foo {}");
        let t = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "type_use" && o.name == "Foo");
        assert!(t.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn import_use_occurrence_emitted() {
        let b = run_ts(r#"import { foo } from "./x";"#);
        let u = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "import_use" && o.name == "foo");
        assert!(u.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn property_name_emits_no_occurrence() {
        let b = run_ts("function f() { const x = obj.foo; }");
        // `obj` should be a read; `foo` should NOT appear as an occurrence.
        assert!(
            b.occurrences
                .iter()
                .any(|o| o.name == "obj" && o.occurrence_kind == "read")
        );
        assert!(
            !b.occurrences.iter().any(|o| o.name == "foo"),
            "property name `foo` should not emit an occurrence, got: {:?}",
            b.occurrences
        );
    }

    #[test]
    fn js_emits_no_type_use() {
        let b = run_js("function f(x) { return x; }");
        assert!(
            !b.occurrences
                .iter()
                .any(|o| o.occurrence_kind == "type_use"),
            "JS should emit no type_use, got: {:?}",
            b.occurrences
        );
    }

    #[test]
    fn js_call_occurrence_emitted() {
        let b = run_js("function f() { g(); } function g() {}");
        let c = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "call" && o.name == "g");
        assert!(c.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn block_scope_in_for_loop() {
        let b = run_ts("function f() { for (let i = 0; i < 3; i++) {} }");
        // Expect at least: file, function, block (for_statement).
        let block_count = b.scopes.iter().filter(|s| s.kind == "block").count();
        assert!(
            block_count >= 1,
            "expected ≥1 block scope, got: {:?}",
            b.scopes
        );
        let i_binding = b
            .bindings
            .iter()
            .find(|x| x.name == "i" && x.binding_kind == "definition");
        assert!(
            i_binding.is_some(),
            "expected i binding, got: {:?}",
            b.bindings
        );
    }

    #[test]
    fn destructuring_param_bindings() {
        let b = run_ts("function f({ x, y: z }, [a, b]) {}");
        let names: Vec<&str> = b
            .bindings
            .iter()
            .filter(|x| x.binding_kind == "parameter")
            .map(|x| x.name.as_str())
            .collect();
        assert!(names.contains(&"x"), "got: {:?}", names);
        assert!(names.contains(&"z"), "got: {:?}", names);
        assert!(names.contains(&"a"), "got: {:?}", names);
        assert!(names.contains(&"b"), "got: {:?}", names);
        // Property key `y` must NOT appear.
        assert!(
            !names.contains(&"y"),
            "property key `y` should not bind, got: {:?}",
            names
        );
    }
}
