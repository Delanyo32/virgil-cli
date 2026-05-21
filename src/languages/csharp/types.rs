//! Issue #13 C# — type-expression / signature / inheritance extractor.
//! Per ADR-0003: full kind decomposition + canonical_name resolution.
//! Contract: docs/types-csharp.md.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::models::{
    FieldTypeRow, InheritanceKind, InheritanceRow, ParameterTypeRow, ReturnsTypeRow, SymbolKind,
    ThrowsRow, TypeRow,
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
    ctx.walk(tree.root_node(), Vec::new());
    ctx.finish()
}

/// Issue #13 followup: extract `throw new X(...)` statements as
/// throws-relation rows. C# has no declared `throws` keyword, so this
/// approximates "thrown by" via runtime `throw_statement` /
/// `throw_expression` nodes nested inside method bodies. The
/// enclosing method's `(name, line, col)` keys the row.
pub fn extract_throws(tree: &Tree, source: &[u8], file_path: &str) -> Vec<ThrowsRow> {
    let mut out = Vec::new();
    walk_throws(tree.root_node(), source, file_path, None, &mut out);
    out
}

fn walk_throws(
    node: Node,
    source: &[u8],
    file_path: &str,
    enclosing: Option<(String, u32, u32)>,
    out: &mut Vec<ThrowsRow>,
) {
    let next_enclosing = match node.kind() {
        "method_declaration"
        | "constructor_declaration"
        | "destructor_declaration"
        | "operator_declaration" => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()))
            .map(|name| {
                // Match symbol IDs, which use the whole declaration's
                // start position, not the name identifier's.
                let (l, c) = node_pos(node);
                (name, l, c)
            })
            .or(enclosing.clone()),
        _ => enclosing.clone(),
    };

    if matches!(node.kind(), "throw_statement" | "throw_expression") {
        if let Some((ref fn_name, fn_line, fn_col)) = next_enclosing {
            // The thrown expression is the first named child of the
            // throw_statement / throw_expression. We only emit rows for
            // `new ExceptionType(...)` forms — re-throw (`throw;`) and
            // variable re-raise (`throw e;`) have no static type and emit
            // nothing.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "object_creation_expression" {
                    if let Some(t) = child.child_by_field_name("type") {
                        let display = render_type(t, source);
                        if !display.is_empty() {
                            out.push(ThrowsRow {
                                file_path: file_path.to_string(),
                                function_start_line: fn_line,
                                function_start_col: fn_col,
                                function_name: fn_name.clone(),
                                function_kind: SymbolKind::Method,
                                exception_display_name: display,
                            });
                        }
                    }
                    break;
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_throws(child, source, file_path, next_enclosing.clone(), out);
    }
}

struct Ctx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    types: Vec<TypeRow>,
    seen_display: HashSet<String>,
    param_types: Vec<ParameterTypeRow>,
    returns_types: Vec<ReturnsTypeRow>,
    inheritance: Vec<InheritanceRow>,
    field_types: Vec<FieldTypeRow>,
    /// `using` directives parsed into `(local_name, canonical_path)` pairs.
    /// Includes both `using A.B.C;` (binds the leaf `C`) and
    /// `using Alias = A.B.C;` (binds `Alias`).
    use_bindings: Vec<UseBinding>,
    /// Full namespace strings as written in the file's `using` block.
    /// Used as fallback fully-qualified prefixes when resolving an
    /// unqualified named type that wasn't matched by a leaf alias.
    using_namespaces: Vec<String>,
    /// Same-file type declarations keyed by simple name → fully-qualified
    /// name (joining the enclosing namespace + nesting type chain).
    same_file_defs: std::collections::HashMap<String, String>,
}

struct UseBinding {
    local_name: String,
    canonical_path: String,
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
            inheritance: Vec::new(),
            field_types: Vec::new(),
            use_bindings: Vec::new(),
            using_namespaces: Vec::new(),
            same_file_defs: std::collections::HashMap::new(),
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
        (
            self.types,
            self.param_types,
            self.returns_types,
            self.inheritance,
            self.field_types,
        )
    }

    /// Pre-pass: collect using directives + same-file type definitions so
    /// the main walk can resolve `canonical_name`.
    fn collect_file_level(&mut self, root: Node) {
        // Using directives (file-level + inside namespaces both count for
        // resolution at file scope).
        self.collect_usings(root);
        // Same-file type defs walk: track namespace + nested-type chain.
        self.collect_defs(root, &[]);
    }

    fn collect_usings(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "using_directive" => self.parse_using(child),
                "namespace_declaration" | "file_scoped_namespace_declaration" => {
                    self.collect_usings(child);
                }
                "compilation_unit" | "declaration_list" => self.collect_usings(child),
                _ => {}
            }
        }
    }

    fn parse_using(&mut self, node: Node) {
        // Shapes:
        //   using System;                 → name field absent, type=System
        //   using static System.Math;     → unnamed "static" keyword child
        //   using Console = System.Console; → name field present (alias)
        //   using A.B.C;
        // The grammar's `using_directive` always has one `type` child.
        let raw = node.utf8_text(self.source).unwrap_or("");
        // `using static …` does not bind type names — skip per
        // docs/types-csharp.md scope-walk step 4.
        if raw.trim_start().starts_with("using static")
            || raw.trim_start().starts_with("global using static")
        {
            return;
        }
        let mut alias: Option<String> = None;
        if let Some(name) = node.child_by_field_name("name")
            && let Ok(s) = name.utf8_text(self.source)
        {
            alias = Some(s.to_string());
        }
        // Find the type child (the one named child that isn't the alias
        // identifier and isn't the "static" keyword).
        let mut type_node: Option<Node> = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    // Could be the alias (if `name` field) — skip if so;
                    // otherwise it's the bare namespace `using X;`.
                    if alias.is_some() {
                        continue;
                    }
                    type_node = Some(child);
                }
                "qualified_name" | "generic_name" | "alias_qualified_name" => {
                    type_node = Some(child);
                }
                _ => {}
            }
        }
        let Some(tn) = type_node else { return };
        let canonical = render_type(tn, self.source);
        if canonical.is_empty() {
            return;
        }
        if let Some(a) = alias {
            self.use_bindings.push(UseBinding {
                local_name: a,
                canonical_path: canonical,
            });
        } else {
            // `using A.B.C;` — binds leaf `C` to `A.B.C` AND records the
            // namespace for fallback prefix-search.
            let leaf = canonical
                .rsplit('.')
                .next()
                .unwrap_or(&canonical)
                .to_string();
            self.use_bindings.push(UseBinding {
                local_name: leaf,
                canonical_path: canonical.clone(),
            });
            self.using_namespaces.push(canonical);
        }
    }

    fn collect_defs(&mut self, node: Node, chain: &[String]) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "namespace_declaration" | "file_scoped_namespace_declaration" => {
                    let ns = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .unwrap_or("");
                    let mut new_chain: Vec<String> = chain.to_vec();
                    if !ns.is_empty() {
                        for seg in ns.split('.') {
                            new_chain.push(seg.to_string());
                        }
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        self.collect_defs(body, &new_chain);
                    } else {
                        // file_scoped_namespace_declaration has no body
                        // field; everything after it lives in that ns. We
                        // recurse the namespace node's own children.
                        let mut nc = child.walk();
                        for c in child.named_children(&mut nc) {
                            if c.kind() != "identifier"
                                && c.kind() != "qualified_name"
                                && c.kind() != "generic_name"
                                && c.kind() != "alias_qualified_name"
                            {
                                self.collect_defs(c, &new_chain);
                            }
                        }
                    }
                }
                "class_declaration"
                | "struct_declaration"
                | "interface_declaration"
                | "record_declaration"
                | "enum_declaration"
                | "delegate_declaration" => {
                    if let Some(name) = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                    {
                        let mut new_chain: Vec<String> = chain.to_vec();
                        new_chain.push(name.to_string());
                        let fqn = new_chain.join(".");
                        self.same_file_defs.insert(name.to_string(), fqn);
                        if let Some(body) = child.child_by_field_name("body") {
                            // Nested types inside this declaration.
                            self.collect_defs(body, &new_chain);
                        }
                    }
                }
                "compilation_unit" | "declaration_list" => {
                    self.collect_defs(child, chain);
                }
                _ => {}
            }
        }
        // file_scoped_namespace_declaration: recurse parent siblings is
        // handled by the caller; nothing extra here.
        let _ = node;
    }

    /// Main walk. `ns_chain` tracks the current namespace path for nested
    /// scope (used when resolving same-file refs in the future).
    fn walk(&mut self, node: Node, ns_chain: Vec<String>) {
        match node.kind() {
            "method_declaration" => self.visit_method(node),
            "constructor_declaration" => self.visit_constructor(node),
            "delegate_declaration" => self.visit_delegate(node),
            "class_declaration"
            | "struct_declaration"
            | "record_declaration"
            | "interface_declaration"
            | "enum_declaration" => self.visit_type_decl(node),
            "property_declaration" => self.visit_property(node),
            "field_declaration" => self.visit_field(node),
            "local_declaration_statement" => self.visit_local(node),
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, ns_chain.clone());
        }
    }

    fn visit_method(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (fn_line, fn_col) = node_pos(name_node);

        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, name, SymbolKind::Method, fn_line, fn_col);
        }
        if let Some(ret) = node.child_by_field_name("returns") {
            let display = render_type(ret, self.source);
            // Skip `void` from generating a row? Contract says `void`
            // canonicalizes to System.Void — we keep the row, matches Rust
            // pilot's behaviour of emitting every annotated return.
            self.emit_type_with_subtree(ret);
            self.returns_types.push(ReturnsTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: name.to_string(),
                function_kind: SymbolKind::Method,
                type_display_name: display,
            });
        }
    }

    fn visit_constructor(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (fn_line, fn_col) = node_pos(name_node);
        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, name, SymbolKind::Method, fn_line, fn_col);
        }
        // Constructors have no return type.
    }

    fn visit_delegate(&mut self, node: Node) {
        // Emit the return type as a type row; treat the delegate's
        // parameter list as parameter-type rows too (delegate signature).
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (fn_line, fn_col) = node_pos(name_node);

        if let Some(ret) = node.child_by_field_name("type") {
            let display = render_type(ret, self.source);
            self.emit_type_with_subtree(ret);
            self.returns_types.push(ReturnsTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: name.to_string(),
                function_kind: SymbolKind::TypeAlias,
                type_display_name: display,
            });
        }
        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, name, SymbolKind::TypeAlias, fn_line, fn_col);
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
            if p.kind() != "parameter" {
                continue;
            }
            let pat_name = p
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(self.source).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let (pl, pc) = node_pos(p.child_by_field_name("name").unwrap_or(p));
            let type_display = if let Some(t) = p.child_by_field_name("type") {
                self.emit_type_with_subtree(t);
                Some(render_type(t, self.source))
            } else {
                None
            };
            let has_default = has_default_value(p);
            self.param_types.push(ParameterTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: fn_name.to_string(),
                function_kind: fn_kind,
                parameter_start_line: pl,
                parameter_start_col: pc,
                parameter_name: pat_name,
                position,
                type_display_name: type_display,
                is_optional: has_default,
                has_default,
            });
            position += 1;
        }
    }

    fn visit_type_decl(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);
        let is_interface = node.kind() == "interface_declaration";
        let is_enum = node.kind() == "enum_declaration";
        let child_kind = match node.kind() {
            "class_declaration" | "record_declaration" => SymbolKind::Class,
            "struct_declaration" => SymbolKind::Struct,
            "interface_declaration" => SymbolKind::Interface,
            _ => SymbolKind::Class,
        };

        // base_list parsing.
        let mut cursor = node.walk();
        for c in node.children(&mut cursor) {
            if c.kind() != "base_list" {
                continue;
            }
            self.collect_inheritance_from_base_list(
                c,
                child_name,
                child_kind,
                cl,
                cc,
                is_interface,
                is_enum,
            );
        }
    }

    fn collect_inheritance_from_base_list(
        &mut self,
        base_list: Node,
        child_name: &str,
        child_kind: SymbolKind,
        cl: u32,
        cc: u32,
        is_interface: bool,
        is_enum: bool,
    ) {
        // Enums: `enum E : byte { ... }` — `byte` is the underlying type,
        // NOT inheritance. Skip emitting inheritance rows for enums (we
        // still emit the type row for the underlying primitive).
        let mut cursor = base_list.walk();
        let mut entries: Vec<Node> = Vec::new();
        for b in base_list.named_children(&mut cursor) {
            // `primary_constructor_base_type` (record `: Base(args)`) and
            // `argument_list` are not type-only entries; the record form
            // wraps a type node we still want to capture.
            match b.kind() {
                "argument_list" => continue,
                _ => entries.push(b),
            }
        }
        if is_enum {
            // Still emit the type rows for the underlying type, but skip
            // inheritance edges.
            for e in entries {
                self.emit_type_with_subtree(e);
            }
            return;
        }
        for (i, entry) in entries.iter().enumerate() {
            // `primary_constructor_base_type` has an inner type child.
            let target = if entry.kind() == "primary_constructor_base_type" {
                entry
                    .named_children(&mut entry.walk())
                    .find(|c| is_type_position_node(c.kind()))
                    .unwrap_or(*entry)
            } else {
                *entry
            };
            let display = render_type(target, self.source);
            if display.is_empty() {
                continue;
            }
            self.emit_type_with_subtree(target);
            let canonical = self.resolve_head(&display);

            // Classify extends vs implements.
            // - Interface declaring inheritance: all are `extends`.
            // - Class/struct/record: per docs, first entry is `extends`
            //   ONLY when it is a class (i.e. NOT an interface).
            //   tree-sitter cannot distinguish — apply the C# naming
            //   convention: identifiers starting with `I` followed by an
            //   uppercase letter are interfaces.
            let kind = if is_interface {
                InheritanceKind::Extends
            } else if i == 0 && !looks_like_interface(&display) {
                InheritanceKind::Extends
            } else {
                InheritanceKind::Implements
            };

            self.inheritance.push(InheritanceRow {
                file_path: self.file_path.to_string(),
                child_start_line: cl,
                child_start_col: cc,
                child_name: child_name.to_string(),
                child_kind,
                parent_display_name: display,
                parent_canonical_name: canonical,
                kind,
            });
        }
    }

    fn visit_property(&mut self, node: Node) {
        let Some(t) = node.child_by_field_name("type") else {
            return;
        };
        self.emit_type_with_subtree(t);
        // Issue #14 + #18.1: emit a field_type row keyed off the
        // property_declaration's own start position so symbol_id
        // matches the Symbol row.
        if let Some(name_node) = node.child_by_field_name("name")
            && let Ok(field_name) = name_node.utf8_text(self.source)
        {
            let (line, col) = node_pos(node);
            self.field_types.push(FieldTypeRow {
                file_path: self.file_path.to_string(),
                field_start_line: line,
                field_start_col: col,
                field_name: field_name.to_string(),
                field_kind: SymbolKind::Field,
                type_display_name: render_type(t, self.source),
            });
        }
    }

    fn visit_field(&mut self, node: Node) {
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() != "variable_declaration" {
                continue;
            }
            let Some(t) = c.child_by_field_name("type") else {
                continue;
            };
            self.emit_type_with_subtree(t);
            // Issue #14: one row per declarator. `int a, b, c;` → 3 rows.
            let display = render_type(t, self.source);
            let mut dc = c.walk();
            for d in c.named_children(&mut dc) {
                if d.kind() != "variable_declarator" {
                    continue;
                }
                if let Some(name_node) = d.child_by_field_name("name")
                    && let Ok(field_name) = name_node.utf8_text(self.source)
                {
                    // #18.1: key on the field_declaration's start
                    // (`node`), not the declarator's, matching what
                    // the symbol query produces as @definition.
                    let (line, col) = node_pos(node);
                    self.field_types.push(FieldTypeRow {
                        file_path: self.file_path.to_string(),
                        field_start_line: line,
                        field_start_col: col,
                        field_name: field_name.to_string(),
                        field_kind: SymbolKind::Field,
                        type_display_name: display.clone(),
                    });
                }
            }
        }
    }

    fn visit_local(&mut self, node: Node) {
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() == "variable_declaration"
                && let Some(t) = c.child_by_field_name("type")
            {
                // Skip `var` — `implicit_type` is never emitted per
                // contract.
                if t.kind() == "implicit_type" {
                    continue;
                }
                self.emit_type_with_subtree(t);
            }
        }
    }

    /// Emit a TypeRow for `node` and meaningful sub-type expressions
    /// nested inside it. Per docs/types-csharp.md.
    fn emit_type_with_subtree(&mut self, node: Node) {
        // `var` is never emitted.
        if node.kind() == "implicit_type" {
            return;
        }
        if let Some((kind, display)) = self.classify_type_node(node) {
            let canonical = self.resolve_for(&display, kind.as_str(), node);
            if self.seen_display.insert(display.clone()) {
                self.types.push(TypeRow {
                    file_path: self.file_path.to_string(),
                    kind,
                    display_name: display,
                    canonical_name: canonical,
                });
            }
        }

        // Recurse into children for nested types. BUT: for
        // `nullable_type`, do NOT emit a separate row for the inner type
        // (per the contract — `DateTime?` does not produce a `DateTime`
        // row in addition to `DateTime?`).
        if node.kind() == "nullable_type" {
            return;
        }
        // For `ref_type`, the wrapped type IS emitted (ref is not a type).
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if !is_type_position_node(c.kind()) {
                // Recurse through transparent containers that wrap types
                // (type_argument_list, array_rank_specifier, etc.).
                if matches!(
                    c.kind(),
                    "type_argument_list" | "tuple_element" | "function_pointer_parameter"
                ) {
                    let mut inner = c.walk();
                    for ic in c.named_children(&mut inner) {
                        if is_type_position_node(ic.kind()) {
                            self.emit_type_with_subtree(ic);
                        }
                    }
                }
                continue;
            }
            self.emit_type_with_subtree(c);
        }
    }

    fn classify_type_node(&self, node: Node) -> Option<(String, String)> {
        let display = render_type(node, self.source);
        if display.is_empty() {
            return None;
        }
        let kind = match node.kind() {
            "predefined_type" => "primitive",
            "identifier" | "qualified_name" | "alias_qualified_name" => "named",
            "generic_name" => "generic",
            "array_type" => "array",
            "tuple_type" => "tuple",
            "nullable_type" => {
                // Carries the inner kind. Inspect the inner type field.
                if let Some(inner) = node.child_by_field_name("type") {
                    return self
                        .classify_type_node(inner)
                        .map(|(_inner_kind, _)| (kind_for_nullable(inner.kind()), display));
                }
                "named"
            }
            "pointer_type" => "named",
            "function_pointer_type" => "function",
            "ref_type" => {
                // Not its own row — recurse into the wrapped type.
                return None;
            }
            "implicit_type" => return None,
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    fn resolve_for(&self, display: &str, kind: &str, node: Node) -> Option<String> {
        match kind {
            "primitive" => {
                let stripped = display.trim_end_matches('?');
                let canon = primitive_canonical(stripped)?;
                if display.ends_with('?') && is_value_type_canonical(&canon) {
                    Some(format!("System.Nullable<{canon}>"))
                } else {
                    Some(canon)
                }
            }
            "array" => {
                // T[] → canonical preserves [] suffix
                let inner = strip_array_suffix(display);
                let inner_canon = self.resolve_head(inner);
                inner_canon.map(|s| {
                    let suffix = &display[inner.len()..];
                    format!("{s}{suffix}")
                })
            }
            "tuple" => {
                // (T1, T2) — resolve each element type via render-walk.
                self.resolve_tuple(node)
            }
            "named" | "generic" => {
                // If the display ends with `?` and the inner display is a
                // resolvable primitive value type, canonicalize through
                // `System.Nullable<T>`.
                if let Some(stripped) = display.strip_suffix('?') {
                    let inner_canon = self.resolve_head(stripped);
                    return match inner_canon {
                        Some(c) if is_value_type_canonical(&c) => {
                            Some(format!("System.Nullable<{c}>"))
                        }
                        // Reference-type NRT annotation: per contract, we
                        // emit canonical = null (unresolved) because we
                        // cannot statically decide.
                        Some(_) => None,
                        None => None,
                    };
                }
                if kind == "generic" {
                    // Resolve head and substitute args.
                    self.resolve_generic(display, node)
                } else {
                    self.resolve_head(display)
                }
            }
            "function" => None,
            _ => None,
        }
    }

    fn resolve_generic(&self, display: &str, node: Node) -> Option<String> {
        // `Name<args...>` — resolve head against scope; substitute args.
        let head = strip_generic_args_csharp(display);
        let head_canon = self.resolve_head(head)?;
        // Walk args (type_argument_list children).
        let arg_list = node
            .named_children(&mut node.walk())
            .find(|c| c.kind() == "type_argument_list");
        let Some(arg_list) = arg_list else {
            return Some(head_canon);
        };
        let mut canon_args: Vec<String> = Vec::new();
        let mut cursor = arg_list.walk();
        for a in arg_list.named_children(&mut cursor) {
            let a_display = render_type(a, self.source);
            // Determine kind of a quickly.
            let arg_kind = match a.kind() {
                "predefined_type" => "primitive",
                "generic_name" => "generic",
                "array_type" => "array",
                "tuple_type" => "tuple",
                "nullable_type" => "named",
                _ => "named",
            };
            let resolved = self.resolve_for(&a_display, arg_kind, a);
            canon_args.push(resolved.unwrap_or(a_display));
        }
        Some(format!("{head_canon}<{}>", canon_args.join(", ")))
    }

    fn resolve_tuple(&self, node: Node) -> Option<String> {
        let mut elem_strs: Vec<String> = Vec::new();
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() != "tuple_element" {
                continue;
            }
            let Some(t) = c.child_by_field_name("type") else {
                continue;
            };
            let t_display = render_type(t, self.source);
            let t_kind = match t.kind() {
                "predefined_type" => "primitive",
                "generic_name" => "generic",
                "array_type" => "array",
                "tuple_type" => "tuple",
                _ => "named",
            };
            let resolved = self.resolve_for(&t_display, t_kind, t).unwrap_or(t_display);
            let elem_name = c
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(self.source).ok());
            let s = match elem_name {
                Some(n) if !n.is_empty() => format!("{resolved} {n}"),
                _ => resolved,
            };
            elem_strs.push(s);
        }
        if elem_strs.is_empty() {
            return None;
        }
        Some(format!("({})", elem_strs.join(", ")))
    }

    /// Resolve a textual head (e.g. `List`, `System.String`, `Foo`) to a
    /// canonical name via the scope walk in docs/types-csharp.md.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = display.trim();
        if head.is_empty() {
            return None;
        }
        // Strip trailing `?` and `[]` and generic args for resolution.
        let head = head.trim_end_matches('?');
        let head = strip_array_suffix(head);
        let head = strip_generic_args_csharp(head);
        let head = head.trim();
        if head.is_empty() {
            return None;
        }
        // Predefined keywords → CLR canonical.
        if let Some(p) = primitive_canonical(head) {
            return Some(p);
        }
        // Qualified — leftmost segment goes through the scope walk.
        let mut segments: Vec<&str> = head.split('.').collect();
        let first = segments[0];
        let rest = if segments.len() > 1 {
            segments.split_off(1).join(".")
        } else {
            String::new()
        };
        // 1. Alias match (`using Alias = X.Y.Z;` or leaf-binding from
        //    plain `using A.B.C;`).
        for u in &self.use_bindings {
            if u.local_name == first {
                let canonical = if rest.is_empty() {
                    u.canonical_path.clone()
                } else {
                    format!("{}.{rest}", u.canonical_path)
                };
                return Some(canonical);
            }
        }
        // 2. Same-file definition (any namespace nesting).
        if let Some(fqn) = self.same_file_defs.get(first) {
            return Some(if rest.is_empty() {
                fqn.clone()
            } else {
                format!("{fqn}.{rest}")
            });
        }
        // 3. Already fully-qualified-looking (more than one segment) →
        //    keep verbatim. Common case: `System.String`.
        if !rest.is_empty() {
            return Some(format!("{first}.{rest}"));
        }
        // 4. Type-parameter heuristic — leave unresolved.
        if is_likely_type_parameter(first) {
            return None;
        }
        // 5. BCL well-known names under `using System;` (and other common
        //    using-namespaces). Without a workspace index we can't verify
        //    arbitrary types live under a using, so we restrict the
        //    fallback to a known set of BCL identifiers. Contract Example
        //    5 (`DateTime` → `System.DateTime` via `using System;`)
        //    depends on this.
        if let Some(canon) = bcl_canonical_under_usings(first, &self.using_namespaces) {
            return Some(canon);
        }
        None
    }
}

fn looks_like_interface(display: &str) -> bool {
    // Strip generic args and namespace qualification to look at the bare
    // type name's first two characters.
    let head = strip_generic_args_csharp(display);
    let name = head.rsplit('.').next().unwrap_or(head);
    let mut chars = name.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('I'), Some(c)) if c.is_ascii_uppercase()
    )
}

fn is_likely_type_parameter(name: &str) -> bool {
    // C# convention: single uppercase letter, optionally followed by
    // digits or a trailing word; the canonical case is `T`, `T1`, `TKey`.
    // We only treat single uppercase or `T` + uppercase letter (e.g.
    // `TKey`) as type parameters.
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if bytes.len() == 1 && bytes[0].is_ascii_uppercase() {
        return true;
    }
    if bytes[0] == b'T' && bytes.get(1).is_some_and(|b| b.is_ascii_uppercase()) {
        return true;
    }
    false
}

fn kind_for_nullable(inner_kind: &str) -> String {
    match inner_kind {
        "predefined_type" => "primitive",
        "generic_name" => "generic",
        "array_type" => "array",
        "tuple_type" => "tuple",
        "identifier" | "qualified_name" | "alias_qualified_name" => "named",
        _ => "named",
    }
    .to_string()
}

fn strip_generic_args_csharp(s: &str) -> &str {
    match s.find('<') {
        Some(idx) => s[..idx].trim_end(),
        None => s,
    }
}

fn strip_array_suffix(s: &str) -> &str {
    // Remove trailing array brackets like `[]`, `[][]`, `[,]`.
    let mut end = s.len();
    let bytes = s.as_bytes();
    loop {
        // Find a matching `[...]` at the tail.
        if end == 0 || bytes[end - 1] != b']' {
            break;
        }
        // Walk back to matching `[`.
        let mut depth = 0;
        let mut i = end;
        while i > 0 {
            i -= 1;
            match bytes[i] {
                b']' => depth += 1,
                b'[' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            break;
        }
        end = i;
    }
    s[..end].trim_end()
}

fn has_default_value(p: Node) -> bool {
    // Parameter has a default `= expr` when an `equals_value_clause` or
    // an `expression` child is present.
    let mut cursor = p.walk();
    for c in p.named_children(&mut cursor) {
        match c.kind() {
            "equals_value_clause" | "expression" => return true,
            // Some grammar versions don't have a dedicated child kind; the
            // expression appears directly. Fall through.
            _ => {}
        }
        if c.kind().ends_with("_expression") || c.kind().ends_with("literal") {
            return true;
        }
    }
    false
}

fn is_type_position_node(kind: &str) -> bool {
    matches!(
        kind,
        "predefined_type"
            | "identifier"
            | "qualified_name"
            | "alias_qualified_name"
            | "generic_name"
            | "array_type"
            | "tuple_type"
            | "nullable_type"
            | "pointer_type"
            | "function_pointer_type"
            | "ref_type"
    )
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

/// Re-serialize a type node to the normalised display_name form per
/// docs/types-csharp.md.
fn render_type(node: Node, source: &[u8]) -> String {
    let raw = node.utf8_text(source).unwrap_or("");
    normalize_type_text(raw)
}

fn normalize_type_text(raw: &str) -> String {
    let mut s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Remove whitespace adjacent to angle/parens/brackets and `?`/`*`.
    for tok in [
        "< ", " >", "( ", " )", "[ ", " ]", " .", ". ", " ?", " *", " ,",
    ] {
        let replacement = tok.replace(' ', "");
        while let Some(idx) = s.find(tok) {
            s.replace_range(idx..idx + tok.len(), &replacement);
        }
    }
    // Re-insert single space after `,` when missing.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == ','
            && let Some(next) = chars.peek()
            && *next != ' '
            && *next != '>'
            && *next != ')'
            && *next != ']'
        {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

fn primitive_canonical(name: &str) -> Option<String> {
    let s = match name {
        "bool" => "System.Boolean",
        "byte" => "System.Byte",
        "sbyte" => "System.SByte",
        "char" => "System.Char",
        "decimal" => "System.Decimal",
        "double" => "System.Double",
        "float" => "System.Single",
        "int" => "System.Int32",
        "uint" => "System.UInt32",
        "long" => "System.Int64",
        "ulong" => "System.UInt64",
        "short" => "System.Int16",
        "ushort" => "System.UInt16",
        "object" => "System.Object",
        "string" => "System.String",
        "void" => "System.Void",
        "nint" => "System.IntPtr",
        "nuint" => "System.UIntPtr",
        "dynamic" => "System.Object",
        _ => return None,
    };
    Some(s.to_string())
}

/// BCL well-known type names that live directly under a specific
/// namespace. Returns the fully-qualified canonical name when `name` is
/// such a type AND the containing namespace is present in
/// `usings`. Restricted to a curated set so that arbitrary identifiers
/// don't get spuriously canonicalized.
fn bcl_canonical_under_usings(name: &str, usings: &[String]) -> Option<String> {
    let ns = match name {
        // System
        "DateTime"
        | "DateTimeOffset"
        | "TimeSpan"
        | "Guid"
        | "Exception"
        | "Action"
        | "Func"
        | "Predicate"
        | "Comparison"
        | "EventHandler"
        | "EventArgs"
        | "Uri"
        | "Version"
        | "Random"
        | "Convert"
        | "Math"
        | "Environment"
        | "Console"
        | "Tuple"
        | "ValueTuple"
        | "IDisposable"
        | "IComparable"
        | "IEquatable"
        | "IFormattable"
        | "Nullable"
        | "Lazy"
        | "WeakReference"
        | "Type"
        | "Activator"
        | "Attribute"
        | "AttributeUsageAttribute"
        | "FlagsAttribute"
        | "ObsoleteAttribute"
        | "NotImplementedException"
        | "ArgumentException"
        | "ArgumentNullException"
        | "ArgumentOutOfRangeException"
        | "InvalidOperationException"
        | "NullReferenceException" => "System",
        // System.Collections.Generic
        "List"
        | "Dictionary"
        | "HashSet"
        | "Queue"
        | "Stack"
        | "LinkedList"
        | "SortedList"
        | "SortedDictionary"
        | "SortedSet"
        | "KeyValuePair"
        | "IEnumerable"
        | "IEnumerator"
        | "ICollection"
        | "IList"
        | "IDictionary"
        | "ISet"
        | "IReadOnlyCollection"
        | "IReadOnlyList"
        | "IReadOnlyDictionary" => "System.Collections.Generic",
        // System.Threading.Tasks
        "Task"
        | "ValueTask"
        | "TaskCompletionSource"
        | "CancellationToken"
        | "CancellationTokenSource" => "System.Threading.Tasks",
        // System.IO
        "Stream" | "FileStream" | "MemoryStream" | "StreamReader" | "StreamWriter"
        | "TextReader" | "TextWriter" | "Path" | "File" | "Directory" | "FileInfo"
        | "DirectoryInfo" => "System.IO",
        // System.Linq
        "IQueryable" | "IGrouping" | "ILookup" | "IOrderedEnumerable" | "IOrderedQueryable" => {
            "System.Linq"
        }
        // Microsoft.AspNetCore.Mvc (common controller types)
        "IActionResult" | "ActionResult" | "ControllerBase" | "Controller" => {
            "Microsoft.AspNetCore.Mvc"
        }
        _ => return None,
    };
    if usings.iter().any(|u| u == ns) {
        Some(format!("{ns}.{name}"))
    } else {
        None
    }
}

/// Value-type CLR canonical names (those eligible for `System.Nullable<>`
/// wrapping under value-type-nullable `T?` syntax).
fn is_value_type_canonical(canon: &str) -> bool {
    matches!(
        canon,
        "System.Boolean"
            | "System.Byte"
            | "System.SByte"
            | "System.Char"
            | "System.Decimal"
            | "System.Double"
            | "System.Single"
            | "System.Int32"
            | "System.UInt32"
            | "System.Int64"
            | "System.UInt64"
            | "System.Int16"
            | "System.UInt16"
            | "System.IntPtr"
            | "System.UIntPtr"
            | "System.DateTime"
            | "System.DateTimeOffset"
            | "System.TimeSpan"
            | "System.Guid"
    )
}

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
        let mut parser = create_parser(Language::CSharp).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let src = r#"
            public class C {
                public int Add(int a, int b) { return a + b; }
            }"#;
        let (types, params, returns, _, _) = run(src, "test.cs");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "int" && t.kind == "primitive"),
            "missing int type row: {:?}",
            types
        );
        let int_row = types.iter().find(|t| t.display_name == "int").unwrap();
        assert_eq!(int_row.canonical_name.as_deref(), Some("System.Int32"));
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "a");
        assert_eq!(params[0].position, 0);
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
        assert_eq!(returns[0].function_name, "Add");
    }

    #[test]
    fn generic_return() {
        let src = r#"
            using System.Collections.Generic;
            public class C {
                public List<int> GetNumbers() { return null; }
            }"#;
        let (types, _, returns, _, _) = run(src, "test.cs");
        let outer = types
            .iter()
            .find(|t| t.display_name == "List<int>")
            .expect("outer");
        assert_eq!(outer.kind, "generic");
        assert_eq!(
            outer.canonical_name.as_deref(),
            Some("System.Collections.Generic.List<System.Int32>")
        );
        assert_eq!(returns[0].type_display_name, "List<int>");
    }

    #[test]
    fn array_type() {
        let src = r#"public class C { public string[] Items { get; set; } }"#;
        let (types, _, _, _, _) = run(src, "test.cs");
        let arr = types
            .iter()
            .find(|t| t.display_name == "string[]")
            .expect("arr row");
        assert_eq!(arr.kind, "array");
        assert_eq!(arr.canonical_name.as_deref(), Some("System.String[]"));
        let inner = types
            .iter()
            .find(|t| t.display_name == "string")
            .expect("inner row");
        assert_eq!(inner.kind, "primitive");
    }

    #[test]
    fn nullable_value_type() {
        let src = r#"
            using System;
            public class C { public DateTime? UpdatedAt { get; set; } }"#;
        let (types, _, _, _, _) = run(src, "test.cs");
        let row = types
            .iter()
            .find(|t| t.display_name == "DateTime?")
            .expect("nullable row");
        // Inner kind=named → DateTime? kind = named (value-type nullable
        // structurally, but display preserves `?`).
        assert_eq!(row.kind, "named");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("System.Nullable<System.DateTime>")
        );
        // No bare `DateTime` row — only the nullable form per contract.
        assert!(
            !types.iter().any(|t| t.display_name == "DateTime"),
            "unexpected bare DateTime row: {:?}",
            types
        );
    }

    #[test]
    fn nullable_primitive() {
        let src = r#"public class C { public int? Count { get; set; } }"#;
        let (types, _, _, _, _) = run(src, "test.cs");
        let row = types
            .iter()
            .find(|t| t.display_name == "int?")
            .expect("int? row");
        assert_eq!(row.kind, "primitive");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("System.Nullable<System.Int32>")
        );
    }

    #[test]
    fn tuple_type() {
        let src = r#"
            public class C {
                public (int Id, string Name) GetTuple() { return (0, ""); }
            }"#;
        let (types, _, returns, _, _) = run(src, "test.cs");
        let row = types.iter().find(|t| t.kind == "tuple").expect("tuple row");
        assert_eq!(row.display_name, "(int Id, string Name)");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("(System.Int32 Id, System.String Name)")
        );
        assert!(returns.iter().any(|r| r.type_display_name.starts_with('(')));
    }

    #[test]
    fn class_extends_and_implements() {
        let src = r#"
            public class Foo : Bar, IFoo, IBar { }
        "#;
        let (_, _, _, inh, _) = run(src, "test.cs");
        let extends: Vec<&str> = inh
            .iter()
            .filter(|r| r.child_name == "Foo" && r.kind == InheritanceKind::Extends)
            .map(|r| r.parent_display_name.as_str())
            .collect();
        let implements: Vec<&str> = inh
            .iter()
            .filter(|r| r.child_name == "Foo" && r.kind == InheritanceKind::Implements)
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert_eq!(extends, vec!["Bar"]);
        assert!(implements.contains(&"IFoo"));
        assert!(implements.contains(&"IBar"));
    }

    #[test]
    fn class_implements_only() {
        // First base entry begins with `I` → treated as interface.
        let src = r#"public class Foo : IFoo, IBar { }"#;
        let (_, _, _, inh, _) = run(src, "test.cs");
        for row in &inh {
            assert_eq!(
                row.kind,
                InheritanceKind::Implements,
                "expected all implements, got {:?}",
                row
            );
        }
    }

    #[test]
    fn interface_extends_only() {
        let src = r#"public interface ISub : ISuper, IOther { }"#;
        let (_, _, _, inh, _) = run(src, "test.cs");
        for row in &inh {
            assert_eq!(row.kind, InheritanceKind::Extends);
            assert_eq!(row.child_name, "ISub");
            assert_eq!(row.child_kind, SymbolKind::Interface);
        }
        let parents: Vec<&str> = inh.iter().map(|r| r.parent_display_name.as_str()).collect();
        assert!(parents.contains(&"ISuper"));
        assert!(parents.contains(&"IOther"));
    }

    #[test]
    fn enum_underlying_type_not_inheritance() {
        let src = r#"public enum Color : byte { Red, Green, Blue }"#;
        let (types, _, _, inh, _) = run(src, "test.cs");
        // No inheritance rows for enums.
        assert!(
            inh.is_empty(),
            "enums should not emit inheritance: {:?}",
            inh
        );
        // The underlying type IS captured as a type row.
        assert!(types.iter().any(|t| t.display_name == "byte"));
    }

    #[test]
    fn using_alias_resolution() {
        let src = r#"
            using Foo = System.Collections.Generic.List<int>;
            public class C { public Foo Items { get; set; } }
        "#;
        let (types, _, _, _, _) = run(src, "test.cs");
        let row = types
            .iter()
            .find(|t| t.display_name == "Foo")
            .expect("Foo row");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("System.Collections.Generic.List<int>")
        );
    }

    #[test]
    fn same_file_canonical() {
        let src = r#"
            namespace MyApp.Models {
                public class User { }
                public class Project { public User Owner { get; set; } }
            }
        "#;
        let (types, _, _, _, _) = run(src, "test.cs");
        let row = types
            .iter()
            .find(|t| t.display_name == "User")
            .expect("User row");
        assert_eq!(row.canonical_name.as_deref(), Some("MyApp.Models.User"));
    }

    #[test]
    fn var_local_not_emitted() {
        let src = r#"
            public class C { public void M() { var x = 1; } }
        "#;
        let (types, _, _, _, _) = run(src, "test.cs");
        // No type row for `var`.
        assert!(!types.iter().any(|t| t.display_name == "var"));
    }

    #[test]
    fn type_parameter_unresolved() {
        let src = r#"public class Box<T> { public T Value { get; set; } }"#;
        let (types, _, _, _, _) = run(src, "test.cs");
        let t_row = types.iter().find(|t| t.display_name == "T").expect("T row");
        assert!(
            t_row.canonical_name.is_none(),
            "type parameter T should be unresolved, got {:?}",
            t_row.canonical_name
        );
    }

    #[test]
    fn parameter_default_value() {
        let src = r#"
            public class C { public void M(int x = 5) { } }
        "#;
        let (_, params, _, _, _) = run(src, "test.cs");
        let p = params
            .iter()
            .find(|p| p.parameter_name == "x")
            .expect("param");
        assert!(p.has_default, "expected has_default for x");
        assert!(p.is_optional);
    }

    #[test]
    fn nested_generic_resolves() {
        let src = r#"
            using System.Collections.Generic;
            public class C {
                public Dictionary<string, List<int>> Lookup() { return null; }
            }
        "#;
        let (types, _, returns, _, _) = run(src, "test.cs");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "Dictionary<string, List<int>>" && t.kind == "generic"),
            "missing outer generic in {:?}",
            types.iter().map(|t| &t.display_name).collect::<Vec<_>>()
        );
        assert!(
            returns[0]
                .type_display_name
                .contains("Dictionary<string, List<int>>")
        );
    }
}
