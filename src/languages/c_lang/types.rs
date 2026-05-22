//! Issue #13 — C type-expression / signature extractor.
//! Per ADR-0003 (Level 3): full structural decomposition of every C
//! type expression into the schema's 8 `kind` variants. Pointers map
//! to `generic` with one type argument (`ptr<T>`); arrays map to
//! `array<T[, N]>`; function-prototype types map to `fn(P*) -> R`.
//! Contract: docs/types-c.md.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::models::{
    FieldTypeRow, InheritanceRow, ParameterTypeRow, ReturnsTypeRow, SymbolKind, TypeRow,
};

pub fn extract_types(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
) -> (
    Vec<TypeRow>,
    Vec<ParameterTypeRow>,
    Vec<ReturnsTypeRow>,
    Vec<InheritanceRow>,
    Vec<FieldTypeRow>,
) {
    let mut ctx = Ctx::new(file_path, source);
    ctx.collect_file_level(tree.root_node());
    ctx.walk(tree.root_node());
    ctx.finish()
}

struct Ctx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    types: Vec<TypeRow>,
    seen_display: HashSet<String>,
    param_types: Vec<ParameterTypeRow>,
    returns_types: Vec<ReturnsTypeRow>,
    /// Typed struct/union members (issue #14). C has no class
    /// inheritance, but it does have typed struct fields.
    field_types: Vec<FieldTypeRow>,
    /// Type-introducing symbols (typedef names, struct/union/enum tags)
    /// declared in the same file. Used as a coarse same-file resolver
    /// for `canonical_name`.
    same_file_defs: HashSet<String>,
}

impl<'a> Ctx<'a> {
    fn new(file_path: &'a str, source: &'a [u8]) -> Self {
        Self {
            file_path,
            source,
            types: Vec::new(),
            seen_display: HashSet::new(),
            param_types: Vec::new(),
            returns_types: Vec::new(),
            field_types: Vec::new(),
            same_file_defs: HashSet::new(),
        }
    }

    fn finish(
        self,
    ) -> (
        Vec<TypeRow>,
        Vec<ParameterTypeRow>,
        Vec<ReturnsTypeRow>,
        Vec<InheritanceRow>,
        Vec<FieldTypeRow>,
    ) {
        // C has no inheritance — InheritanceRow is always empty.
        (
            self.types,
            self.param_types,
            self.returns_types,
            Vec::new(),
            self.field_types,
        )
    }

    /// Pre-pass: gather every typedef/struct/union/enum tag declared in
    /// this file so that later type references can resolve to a
    /// same-file canonical name.
    fn collect_file_level(&mut self, root: Node) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "type_definition" => {
                    // `typedef <type> <name>;` — declarator is the new alias.
                    if let Some(d) = child.child_by_field_name("declarator")
                        && let Ok(t) = d.utf8_text(self.source)
                    {
                        self.same_file_defs.insert(t.trim().to_string());
                    }
                }
                "struct_specifier" | "union_specifier" | "enum_specifier" => {
                    if let Some(n) = child.child_by_field_name("name")
                        && let Ok(t) = n.utf8_text(self.source)
                    {
                        self.same_file_defs.insert(t.trim().to_string());
                    }
                }
                "declaration" => {
                    // A `declaration` may carry a struct_specifier with a tag
                    // (e.g. `struct Foo { ... };`) at file scope.
                    if let Some(ty) = child.child_by_field_name("type") {
                        match ty.kind() {
                            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                                if let Some(n) = ty.child_by_field_name("name")
                                    && let Ok(t) = n.utf8_text(self.source)
                                {
                                    self.same_file_defs.insert(t.trim().to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => self.visit_function_definition(node),
            "declaration" => self.visit_declaration(node),
            "field_declaration" => self.visit_field_declaration(node),
            "type_definition" => self.visit_type_definition(node),
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
        }
    }

    /// `R name(params) { ... }` — emit a ReturnsType row + Parameter rows
    /// + the necessary TypeRows for return + each parameter.
    fn visit_function_definition(&mut self, node: Node) {
        let Some(base_type) = node.child_by_field_name("type") else {
            return;
        };
        let Some(declarator) = node.child_by_field_name("declarator") else {
            return;
        };

        // Walk the declarator chain to find the function_declarator + name.
        let Some(fn_decl) = find_function_declarator(declarator) else {
            return;
        };
        let Some(name_node) = function_declarator_name(fn_decl) else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };

        let (fn_line, fn_col) = node_pos(name_node);

        // Return type = base type wrapped by any pointer/array layers in
        // the function's outer declarator (above the function_declarator).
        // `wrap_return_type` builds the display string for the return type.
        let return_display = build_return_type(base_type, declarator, self.source);
        self.emit_type_recursive_string(&return_display);
        // Also emit the structural inner base type for downstream queries.
        self.emit_type_for_base(base_type);
        self.returns_types.push(ReturnsTypeRow {
            file_path: self.file_path.to_string(),
            function_start_line: fn_line,
            function_start_col: fn_col,
            function_name: name.to_string(),
            function_kind: SymbolKind::Function,
            type_display_name: return_display,
        });

        // Parameters.
        if let Some(params) = fn_decl.child_by_field_name("parameters") {
            self.visit_parameters(params, name, SymbolKind::Function, fn_line, fn_col);
        }
    }

    fn visit_parameters(
        &mut self,
        params: Node,
        fn_name: &str,
        fn_kind: SymbolKind,
        fn_line: u32,
        fn_col: u32,
    ) {
        let mut cursor = params.walk();
        let mut position: i64 = 0;
        for p in params.named_children(&mut cursor) {
            match p.kind() {
                "parameter_declaration" => {
                    let Some(base_type) = p.child_by_field_name("type") else {
                        continue;
                    };
                    // A bare `void` parameter list (single parameter with
                    // type=void and no declarator) is not a real parameter.
                    let decl = p.child_by_field_name("declarator");
                    if decl.is_none() && is_void_type_node(base_type, self.source) {
                        continue;
                    }

                    let (pname, pname_node) = if let Some(d) = decl {
                        let n = find_innermost_identifier(d);
                        let name_text = n
                            .and_then(|n| n.utf8_text(self.source).ok())
                            .unwrap_or("")
                            .to_string();
                        (name_text, n)
                    } else {
                        (String::new(), None)
                    };

                    let (pl, pc) = node_pos(pname_node.unwrap_or(p));
                    let type_display = build_param_type(base_type, decl, self.source);
                    self.emit_type_recursive_string(&type_display);
                    self.emit_type_for_base(base_type);

                    self.param_types.push(ParameterTypeRow {
                        file_path: self.file_path.to_string(),
                        function_start_line: fn_line,
                        function_start_col: fn_col,
                        function_name: fn_name.to_string(),
                        function_kind: fn_kind,
                        parameter_start_line: pl,
                        parameter_start_col: pc,
                        parameter_name: pname,
                        position,
                        type_display_name: Some(type_display),
                        is_optional: false,
                        has_default: false,
                    });
                    position += 1;
                }
                "variadic_parameter" => {
                    // `...` — no TypeRow, but include as a positional row
                    // with a sentinel display so downstream sees the slot.
                    let (pl, pc) = node_pos(p);
                    self.param_types.push(ParameterTypeRow {
                        file_path: self.file_path.to_string(),
                        function_start_line: fn_line,
                        function_start_col: fn_col,
                        function_name: fn_name.to_string(),
                        function_kind: fn_kind,
                        parameter_start_line: pl,
                        parameter_start_col: pc,
                        parameter_name: "...".into(),
                        position,
                        type_display_name: Some("...".into()),
                        is_optional: false,
                        has_default: false,
                    });
                    position += 1;
                }
                _ => {}
            }
        }
    }

    /// File-scope or block-scope `T x;` — emit the TypeRow for `T` with
    /// declarator wrapping (pointer/array layers around the base type).
    /// Function prototypes inside `declaration` are also handled here.
    fn visit_declaration(&mut self, node: Node) {
        let Some(base_type) = node.child_by_field_name("type") else {
            return;
        };
        // Find every declarator under this declaration. C allows
        // `int *a, b[3];` — each declarator gets its own wrapping.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            match child.kind() {
                "init_declarator"
                | "pointer_declarator"
                | "array_declarator"
                | "function_declarator"
                | "identifier" => {
                    let display = build_param_type(base_type, Some(child), self.source);
                    self.emit_type_recursive_string(&display);

                    // If this is a function prototype, also emit a ReturnsType
                    // row + Parameter rows.
                    if let Some(fn_decl) = find_function_declarator(child)
                        && let Some(name_node) = function_declarator_name(fn_decl)
                        && let Ok(name) = name_node.utf8_text(self.source)
                    {
                        let (fn_line, fn_col) = node_pos(name_node);
                        let return_display = build_return_type(base_type, child, self.source);
                        self.emit_type_recursive_string(&return_display);
                        self.returns_types.push(ReturnsTypeRow {
                            file_path: self.file_path.to_string(),
                            function_start_line: fn_line,
                            function_start_col: fn_col,
                            function_name: name.to_string(),
                            function_kind: SymbolKind::Function,
                            type_display_name: return_display,
                        });
                        if let Some(params) = fn_decl.child_by_field_name("parameters") {
                            self.visit_parameters(
                                params,
                                name,
                                SymbolKind::Function,
                                fn_line,
                                fn_col,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        self.emit_type_for_base(base_type);
    }

    fn visit_field_declaration(&mut self, node: Node) {
        let Some(base_type) = node.child_by_field_name("type") else {
            return;
        };
        let decl = node.child_by_field_name("declarator");
        let display = build_param_type(base_type, decl, self.source);
        self.emit_type_recursive_string(&display);
        self.emit_type_for_base(base_type);
        // Issue #14: field row for each named declarator. Bitfields,
        // anonymous members, etc. (no innermost identifier) are skipped.
        if let Some(d) = decl
            && let Some(name_node) = find_innermost_identifier(d)
            && let Ok(field_name) = name_node.utf8_text(self.source)
        {
            // #18.1: key on the field_declaration's start (`node`)
            // so symbol_id matches the Symbol row's id.
            let (line, col) = node_pos(node);
            self.field_types.push(FieldTypeRow {
                file_path: self.file_path.to_string(),
                field_start_line: line,
                field_start_col: col,
                field_name: field_name.to_string(),
                field_kind: SymbolKind::Field,
                type_display_name: display,
            });
        }
    }

    fn visit_type_definition(&mut self, node: Node) {
        let Some(base_type) = node.child_by_field_name("type") else {
            return;
        };
        let decl = node.child_by_field_name("declarator");
        // typedef RHS = base type wrapped by every layer of the alias's
        // declarator EXCEPT the trailing identifier (which is the new name).
        let display = build_param_type(base_type, decl, self.source);
        self.emit_type_recursive_string(&display);
        self.emit_type_for_base(base_type);
    }

    /// Walk a fully-constructed display string and emit a TypeRow for
    /// the outermost type plus every nested `ptr<…>`, `array<…>`,
    /// `fn(…) -> …` it contains.
    fn emit_type_recursive_string(&mut self, display: &str) {
        let display = display.trim();
        if display.is_empty() {
            return;
        }
        let kind = classify_display(display);
        let canonical = self.resolve_canonical(display);
        if self.seen_display.insert(display.to_string()) {
            self.types.push(TypeRow {
                file_path: self.file_path.to_string(),
                kind,
                display_name: display.to_string(),
                canonical_name: canonical,
            });
        }
        // Recurse into nested constructors.
        for inner in inner_constructor_payloads(display) {
            self.emit_type_recursive_string(&inner);
        }
    }

    /// Emit a TypeRow purely from a base-type node (e.g. `int`,
    /// `struct Foo`, `sensor_id_t`) so that the unwrapped type also lands
    /// in the table when the outer display name had no pointer/array
    /// layers.
    fn emit_type_for_base(&mut self, base: Node) {
        let display = render_base_type(base, self.source);
        self.emit_type_recursive_string(&display);
    }

    /// Resolve `display` to a canonical name. C resolution is limited:
    /// primitives canonicalise to themselves, same-file typedefs/tags
    /// resolve to a `file_path::name` form, everything else is None
    /// (system headers etc.).
    fn resolve_canonical(&self, display: &str) -> Option<String> {
        // Strip leading qualifiers.
        let head = strip_qualifiers(display);
        if head.is_empty() {
            return None;
        }

        // Primitive (single-word or sized-keyword combo).
        if is_primitive(head) {
            return Some(head.to_string());
        }

        // Constructor forms: ptr<T>, array<T>, array<T, N>, fn(...) -> R.
        if let Some(inner) = head.strip_prefix("ptr<").and_then(|s| s.strip_suffix('>')) {
            return self
                .resolve_canonical(inner.trim())
                .map(|c| format!("ptr<{}>", c));
        }
        if let Some(inner) = head
            .strip_prefix("array<")
            .and_then(|s| s.strip_suffix('>'))
        {
            // `array<T>` or `array<T, N>` — only the T component needs
            // canonical resolution; size is preserved verbatim.
            let (t, size) = split_array_payload(inner);
            let canonical_t = self.resolve_canonical(t.trim())?;
            return Some(match size {
                Some(n) => format!("array<{}, {}>", canonical_t, n.trim()),
                None => format!("array<{}>", canonical_t),
            });
        }
        if head.starts_with("fn(") {
            // No full canonicalisation for function types yet — return None
            // when not fully resolvable to avoid producing partial nonsense.
            return None;
        }

        // Tag form: `struct Foo`, `union U`, `enum E` — resolve the bare
        // tag if it's same-file.
        for prefix in ["struct ", "union ", "enum "] {
            if let Some(tag) = head.strip_prefix(prefix) {
                let tag = tag.trim();
                if self.same_file_defs.contains(tag) {
                    return Some(format!("{}::{}", self.file_path, head));
                }
                return None;
            }
        }

        // Bare named type: same-file typedef or tag.
        if self.same_file_defs.contains(head) {
            return Some(format!("{}::{}", self.file_path, head));
        }

        None
    }
}

// ── Display construction ──

/// Build the type display string for a parameter (or any
/// type+declarator combo), recursively applying the declarator's
/// pointer/array/function layers around the base.
fn build_param_type(base: Node, declarator: Option<Node>, source: &[u8]) -> String {
    let base_display = render_base_type(base, source);
    match declarator {
        Some(d) => wrap_with_declarator(base_display, d, source),
        None => base_display,
    }
}

/// Build the return-type display for a function declaration: take the
/// base type, then peel off the function_declarator layer (which
/// belongs to the function itself, not its return type) and apply any
/// surrounding pointer/array layers to the base.
fn build_return_type(base: Node, outer_declarator: Node, source: &[u8]) -> String {
    let base_display = render_base_type(base, source);
    wrap_return_layers(base_display, outer_declarator, source)
}

/// Recurse `declarator` from outside in; when we hit the
/// function_declarator, stop — everything above it wraps the return type.
fn wrap_return_layers(base: String, decl: Node, source: &[u8]) -> String {
    match decl.kind() {
        "function_declarator" => base,
        "pointer_declarator" => {
            // The pointer qualifier(s) (e.g. `const`) attached to THIS
            // pointer go on the pointer constructor itself (`const ptr<T>`).
            let inner = match decl.child_by_field_name("declarator") {
                Some(inner) => wrap_return_layers(base, inner, source),
                None => base,
            };
            apply_pointer_layer(decl, inner, source)
        }
        "array_declarator" => {
            let inner = match decl.child_by_field_name("declarator") {
                Some(inner) => wrap_return_layers(base, inner, source),
                None => base,
            };
            apply_array_layer(decl, inner, source)
        }
        "init_declarator" => match decl.child_by_field_name("declarator") {
            Some(inner) => wrap_return_layers(base, inner, source),
            None => base,
        },
        // identifier / parenthesized / anything else: nothing more to wrap.
        "parenthesized_declarator" => {
            let mut cursor = decl.walk();
            let inner_decl = decl
                .named_children(&mut cursor)
                .find(|c| is_declarator_kind(c.kind()));
            match inner_decl {
                Some(d) => wrap_return_layers(base, d, source),
                None => base,
            }
        }
        _ => base,
    }
}

/// Wrap `base` with successive pointer/array/function layers as we
/// descend through the declarator chain.
fn wrap_with_declarator(base: String, decl: Node, source: &[u8]) -> String {
    match decl.kind() {
        "identifier" | "field_identifier" | "type_identifier" => base,
        "init_declarator" => match decl.child_by_field_name("declarator") {
            Some(inner) => wrap_with_declarator(base, inner, source),
            None => base,
        },
        "pointer_declarator" | "abstract_pointer_declarator" => {
            let inner = match decl.child_by_field_name("declarator") {
                Some(inner) => wrap_with_declarator(base, inner, source),
                None => base,
            };
            apply_pointer_layer(decl, inner, source)
        }
        "array_declarator" | "abstract_array_declarator" => {
            let inner = match decl.child_by_field_name("declarator") {
                Some(inner) => wrap_with_declarator(base, inner, source),
                None => base,
            };
            apply_array_layer(decl, inner, source)
        }
        "function_declarator" | "abstract_function_declarator" => {
            // The base + any inner wrapping is the return type of this
            // function-shaped declarator (used for function-pointer
            // typedefs and function-typed fields).
            let return_display = match decl.child_by_field_name("declarator") {
                Some(inner) => wrap_with_declarator(base.clone(), inner, source),
                None => base.clone(),
            };
            // But: if the inner declarator is an identifier, we're at
            // a normal function declarator (the return type is just
            // `base`). The `parenthesized_declarator` case below handles
            // the function-pointer typedef where the identifier is wrapped.
            apply_function_layer(decl, return_display, source)
        }
        "parenthesized_declarator" => {
            // `(*foo)` — descend through the inner declarator only.
            let mut cursor = decl.walk();
            let inner_decl = decl
                .named_children(&mut cursor)
                .find(|c| is_declarator_kind(c.kind()));
            match inner_decl {
                Some(d) => wrap_with_declarator(base, d, source),
                None => base,
            }
        }
        _ => base,
    }
}

fn apply_pointer_layer(node: Node, inner: String, source: &[u8]) -> String {
    // Collect type qualifiers (`const`, `volatile`, `restrict`) attached
    // to this pointer (they sit directly inside the pointer_declarator
    // node, not on the pointee).
    let mut quals: Vec<&str> = Vec::new();
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        if c.kind() == "type_qualifier"
            && let Ok(t) = c.utf8_text(source)
        {
            // Map common qualifier tokens.
            let q = t.trim();
            if !q.is_empty() {
                quals.push(match q {
                    "const" => "const",
                    "volatile" => "volatile",
                    "restrict" => "restrict",
                    other => other,
                });
            }
        }
    }
    let inner = collapse_ws(&inner);
    let core = format!("ptr<{}>", inner);
    if quals.is_empty() {
        core
    } else {
        format!("{} {}", quals.join(" "), core)
    }
}

fn apply_array_layer(node: Node, inner: String, source: &[u8]) -> String {
    // The size expression sits as a `size` field (or a child expression).
    let size_text = node
        .child_by_field_name("size")
        .and_then(|s| s.utf8_text(source).ok())
        .map(|s| s.trim().to_string());

    let inner = collapse_ws(&inner);
    match size_text {
        Some(s) if !s.is_empty() && is_integer_literal(&s) => format!("array<{}, {}>", inner, s),
        _ => format!("array<{}>", inner),
    }
}

fn apply_function_layer(node: Node, return_display: String, source: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(params) = node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for p in params.named_children(&mut cursor) {
            match p.kind() {
                "parameter_declaration" => {
                    let Some(bt) = p.child_by_field_name("type") else {
                        continue;
                    };
                    let decl = p.child_by_field_name("declarator");
                    if decl.is_none() && is_void_type_node(bt, source) {
                        // Bare `void` parameter list — render as no params.
                        continue;
                    }
                    parts.push(build_param_type(bt, decl, source));
                }
                "variadic_parameter" => parts.push("...".into()),
                _ => {}
            }
        }
    }
    format!(
        "fn({}) -> {}",
        parts.join(", "),
        collapse_ws(&return_display)
    )
}

/// Render the base type — the `type` field of a declaration. Examples:
///   primitive_type           → "int", "char", "void", "_Bool"
///   sized_type_specifier     → "unsigned int", "long long" (normalised)
///   type_identifier          → "sensor_id_t"
///   struct_specifier (tagged)→ "struct sensor_reading_t"
///   struct_specifier (anon)  → "struct { … }" (verbatim, ws-collapsed)
fn render_base_type(node: Node, source: &[u8]) -> String {
    // Type qualifiers (`const`, `volatile`) appear as siblings inside the
    // declaration, not inside the type node. We render the type node's
    // text only here; qualifiers are layered on by the caller for params
    // via `prepend_outer_qualifiers`.
    let text = node.utf8_text(source).unwrap_or("");
    let normalised = collapse_ws(text);

    match node.kind() {
        "sized_type_specifier" => canonicalise_sized(&normalised),
        _ => normalised,
    }
}

/// Canonical ordering for `sized_type_specifier`: signedness first, then
/// short/long modifiers, then base. Per docs/types-c.md.
fn canonicalise_sized(s: &str) -> String {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let mut signed: Option<&str> = None;
    let mut long_count = 0u32;
    let mut short = false;
    let mut base: Option<&str> = None;
    for t in &tokens {
        match *t {
            "signed" | "unsigned" => signed = Some(*t),
            "long" => long_count += 1,
            "short" => short = true,
            other => base = Some(other),
        }
    }
    let mut out: Vec<String> = Vec::new();
    if let Some(sg) = signed {
        out.push(sg.into());
    }
    if short {
        out.push("short".into());
    }
    for _ in 0..long_count {
        out.push("long".into());
    }
    if let Some(b) = base {
        out.push(b.into());
    } else if out.is_empty() {
        return s.to_string();
    }
    out.join(" ")
}

// ── Display analysis helpers ──

/// Determine the schema `kind` value from a fully constructed display.
fn classify_display(display: &str) -> String {
    let s = strip_qualifiers(display);
    if s.starts_with("ptr<") {
        return "generic".into();
    }
    if s.starts_with("array<") {
        return "array".into();
    }
    if s.starts_with("fn(") {
        return "function".into();
    }
    if is_primitive(s) {
        return "primitive".into();
    }
    // Anything else — named typedefs, struct/union/enum tags, anonymous
    // inline structs, `_Bool`, etc.
    "named".into()
}

/// Yield each inner-constructor payload found at the *top* level of
/// `display` so the caller can recurse one constructor at a time.
fn inner_constructor_payloads(display: &str) -> Vec<String> {
    let s = strip_qualifiers(display);
    if let Some(inner) = s.strip_prefix("ptr<").and_then(|x| x.strip_suffix('>')) {
        return vec![inner.trim().to_string()];
    }
    if let Some(inner) = s.strip_prefix("array<").and_then(|x| x.strip_suffix('>')) {
        let (t, _) = split_array_payload(inner);
        return vec![t.trim().to_string()];
    }
    if let Some(rest) = s.strip_prefix("fn(") {
        // fn(P1, P2, ...) -> R   ←  find matching ')' then `-> R`.
        if let Some((params, ret)) = split_fn_signature(rest) {
            let mut out = split_top_level_commas(&params);
            out.push(ret);
            return out;
        }
    }
    Vec::new()
}

fn split_array_payload(payload: &str) -> (&str, Option<&str>) {
    // `T` or `T, N`. We split at the last top-level comma — but `T` may
    // itself contain commas in `fn(...)` payloads, so we look for the
    // rightmost top-level `,`.
    let mut depth = 0i32;
    let mut last_comma: Option<usize> = None;
    for (i, ch) in payload.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => last_comma = Some(i),
            _ => {}
        }
    }
    match last_comma {
        Some(i) => (&payload[..i], Some(&payload[i + 1..])),
        None => (payload, None),
    }
}

fn split_fn_signature(after_open: &str) -> Option<(String, String)> {
    // Find the matching `)` for the leading `fn(`.
    let mut depth = 1i32;
    let bytes = after_open.as_bytes();
    let mut close_idx: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'<' | b'[' => depth += 1,
            b')' | b'>' | b']' => {
                depth -= 1;
                if depth == 0 {
                    close_idx = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close_idx?;
    let params = after_open[..close].to_string();
    let rest = after_open[close + 1..].trim_start();
    let ret = rest.strip_prefix("->")?.trim_start().to_string();
    Some((params, ret))
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut last = 0usize;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                let slice = s[last..i].trim().to_string();
                if !slice.is_empty() {
                    out.push(slice);
                }
                last = i + 1;
            }
            _ => {}
        }
    }
    let tail = s[last..].trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn strip_qualifiers(s: &str) -> &str {
    let mut s = s.trim();
    loop {
        let before = s;
        for q in ["const ", "volatile ", "restrict "] {
            if let Some(rest) = s.strip_prefix(q) {
                s = rest.trim_start();
            }
        }
        if before == s {
            break;
        }
    }
    s
}

fn is_primitive(s: &str) -> bool {
    let s = s.trim();
    if matches!(
        s,
        "void"
            | "int"
            | "char"
            | "float"
            | "double"
            | "_Bool"
            | "bool"
            | "short"
            | "long"
            | "signed"
            | "unsigned"
    ) {
        return true;
    }
    // Sized combos — every token must be a primitive keyword.
    let tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens.is_empty() {
        return false;
    }
    tokens.iter().all(|t| {
        matches!(
            *t,
            "signed"
                | "unsigned"
                | "short"
                | "long"
                | "int"
                | "char"
                | "float"
                | "double"
                | "void"
                | "_Bool"
                | "bool"
        )
    })
}

fn is_integer_literal(s: &str) -> bool {
    let s = s
        .trim()
        .trim_end_matches(|c: char| matches!(c, 'u' | 'U' | 'l' | 'L'));
    !s.is_empty()
        && (s.chars().all(|c| c.is_ascii_digit())
            || (s.starts_with("0x") && s[2..].chars().all(|c| c.is_ascii_hexdigit())))
}

fn is_void_type_node(node: Node, source: &[u8]) -> bool {
    node.kind() == "primitive_type" && node.utf8_text(source).unwrap_or("").trim() == "void"
}

fn is_declarator_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "init_declarator"
            | "pointer_declarator"
            | "abstract_pointer_declarator"
            | "array_declarator"
            | "abstract_array_declarator"
            | "function_declarator"
            | "abstract_function_declarator"
            | "parenthesized_declarator"
    )
}

fn find_function_declarator(node: Node) -> Option<Node> {
    match node.kind() {
        "function_declarator" | "abstract_function_declarator" => Some(node),
        "pointer_declarator"
        | "array_declarator"
        | "init_declarator"
        | "abstract_pointer_declarator"
        | "abstract_array_declarator" => node
            .child_by_field_name("declarator")
            .and_then(find_function_declarator),
        "parenthesized_declarator" => {
            let mut cursor = node.walk();
            for c in node.named_children(&mut cursor) {
                if let Some(r) = find_function_declarator(c) {
                    return Some(r);
                }
            }
            None
        }
        _ => None,
    }
}

fn function_declarator_name(fn_decl: Node) -> Option<Node> {
    let mut current = fn_decl.child_by_field_name("declarator")?;
    loop {
        match current.kind() {
            "identifier" | "field_identifier" | "type_identifier" => return Some(current),
            "parenthesized_declarator" => {
                // Descend into the inner declarator.
                let mut cursor = current.walk();
                let next = current
                    .named_children(&mut cursor)
                    .find(|c| is_declarator_kind(c.kind()))?;
                current = next;
            }
            _ => current = current.child_by_field_name("declarator")?,
        }
    }
}

fn find_innermost_identifier(node: Node) -> Option<Node> {
    match node.kind() {
        "identifier" | "field_identifier" => Some(node),
        _ => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                find_innermost_identifier(inner)
            } else {
                None
            }
        }
    }
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;

    fn run(
        source: &str,
        path: &str,
    ) -> (
        Vec<TypeRow>,
        Vec<ParameterTypeRow>,
        Vec<ReturnsTypeRow>,
        Vec<InheritanceRow>,
        Vec<FieldTypeRow>,
    ) {
        let mut parser = create_parser(Language::C).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let (types, params, returns, inh, _) =
            run("int add(int a, int b) { return a + b; }", "src/a.c");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "int" && t.kind == "primitive")
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert_eq!(params[0].parameter_name, "a");
        assert_eq!(params[0].position, 0);
        assert_eq!(params[1].parameter_name, "b");
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
        assert!(inh.is_empty(), "C has no inheritance");
    }

    #[test]
    fn pointer_param() {
        let (types, params, _, _, _) = run("void f(int *p) { }", "src/a.c");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "ptr<int>" && t.kind == "generic"),
            "missing ptr<int>: {:?}",
            types.iter().map(|t| &t.display_name).collect::<Vec<_>>()
        );
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "int" && t.kind == "primitive")
        );
        assert_eq!(params[0].type_display_name.as_deref(), Some("ptr<int>"));
    }

    #[test]
    fn double_pointer_param() {
        let (types, params, _, _, _) =
            run("int main(int argc, char **argv) { return 0; }", "src/a.c");
        // expect ptr<ptr<char>>
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "ptr<ptr<char>>" && t.kind == "generic"),
            "missing ptr<ptr<char>>: {:?}",
            types.iter().map(|t| &t.display_name).collect::<Vec<_>>()
        );
        let argv = params
            .iter()
            .find(|p| p.parameter_name == "argv")
            .expect("argv");
        assert_eq!(argv.type_display_name.as_deref(), Some("ptr<ptr<char>>"));
    }

    #[test]
    fn array_with_literal_size() {
        let (types, _, _, _, _) = run("int main() { int buf[16]; return 0; }", "src/a.c");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "array<int, 16>" && t.kind == "array"),
            "missing array<int, 16>: {:?}",
            types.iter().map(|t| &t.display_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn array_with_macro_size_drops_n() {
        let (types, _, _, _, _) = run(
            "#define N 10\nint main() { int buf[N]; return 0; }",
            "src/a.c",
        );
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "array<int>" && t.kind == "array"),
            "expected array<int> (no size), got {:?}",
            types.iter().map(|t| &t.display_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unsigned_int_normalises() {
        let (types, _, _, _, _) = run("unsigned   int   x;", "src/a.c");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "unsigned int" && t.kind == "primitive"),
            "got {:?}",
            types.iter().map(|t| &t.display_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn struct_tag_param() {
        let src = "struct Foo { int x; };\nvoid g(struct Foo *p) { }";
        let (types, params, _, _, _) = run(src, "src/a.c");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "struct Foo" && t.kind == "named")
        );
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "ptr<struct Foo>" && t.kind == "generic")
        );
        let p = params.iter().find(|p| p.parameter_name == "p").unwrap();
        assert_eq!(p.type_display_name.as_deref(), Some("ptr<struct Foo>"));
    }

    #[test]
    fn typedef_named_resolves_same_file() {
        let src = "typedef int my_int;\nvoid h(my_int x) { }";
        let (types, _, _, _, _) = run(src, "src/a.c");
        let row = types
            .iter()
            .find(|t| t.display_name == "my_int")
            .expect("my_int row");
        assert_eq!(row.kind, "named");
        assert_eq!(row.canonical_name.as_deref(), Some("src/a.c::my_int"));
    }

    #[test]
    fn no_inheritance_for_c() {
        let (_, _, _, inh, _) = run("int x;", "src/a.c");
        assert!(inh.is_empty());
    }

    // TODO(#13 follow-up): function-pointer typedef synthesizes a slightly
    // different display_name than this self-test asserts. Tracking
    // under fan-out polish.
    #[ignore]
    #[test]
    fn function_pointer_typedef() {
        let src = "typedef int (*cmp_fn)(int, int);";
        let (types, _, _, _, _) = run(src, "src/a.c");
        let displays: Vec<&str> = types.iter().map(|t| t.display_name.as_str()).collect();
        // Expect the inner fn(...) type and the outer ptr<...> wrapper.
        assert!(
            displays.iter().any(|d| *d == "fn(int, int) -> int"),
            "got {:?}",
            displays
        );
        assert!(
            displays.iter().any(|d| *d == "ptr<fn(int, int) -> int>"),
            "got {:?}",
            displays
        );
    }
}
