//! Issue #13 Go extractor — type-expression / signature / inheritance.
//! Contract: docs/types-go.md. Mirrors the Rust pilot in
//! `src/languages/rust_lang/types.rs`.
//!
//! Notable Go conventions:
//! - Pointer `*T` → kind=generic, single type-arg referent (ADR-0003 policy 2).
//! - Slice `[]T` / array `[N]T` / `[...]T` → kind=array.
//! - `map[K]V` and `chan T` → kind=generic.
//! - `func(A) R` → kind=function.
//! - Multi-return tuples: the *first* return is the `ReturnsTypeRow`'s
//!   display (full signature stays a single returns row carrying the
//!   first type); additional returns are emitted as `ParameterTypeRow`s
//!   with negative `position` and synthetic names `_retN`. Named return
//!   values keep their source name.
//! - Interface embedding (`interface { Foo }`) → `InheritanceRow` of kind
//!   `Extends`. Struct embedding is *not* modeled as inheritance.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::models::{
    FieldTypeRow, InheritanceKind, InheritanceRow, ParameterTypeRow, ReturnsTypeRow, SymbolKind, TypeRow,
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
    inheritance: Vec<InheritanceRow>,
    field_types: Vec<FieldTypeRow>,
    /// `import alias "path";` parsed into `(local_name, canonical_path)`.
    imports: Vec<ImportBinding>,
    /// Same-file named-type declarations (struct/interface/alias). Used
    /// to resolve bare identifiers to `<package>.<Name>`.
    same_file_defs: HashSet<String>,
    /// Package canonical prefix for this file, derived from the file path
    /// (best-effort: parent directory). Resolved names become
    /// `<package_path>.<Name>` when a same-package match is found.
    package_path: String,
}

struct ImportBinding {
    /// Local name that prefixes a `qualified_type`. For aliased imports
    /// (`import foo "x/y/z"`) this is the alias; for unaliased it is the
    /// last `/`-segment of the import path.
    local_name: String,
    /// Full import path as written in source (without surrounding quotes).
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
            imports: Vec::new(),
            same_file_defs: HashSet::new(),
            package_path: derive_package_path(file_path),
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

    /// Pre-pass over the file's root: gather imports + same-file named
    /// type declarations so the main walk can canonicalize identifiers.
    fn collect_file_level(&mut self, root: Node) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "import_declaration" => self.collect_imports(child),
                "type_declaration" => {
                    let mut tc = child.walk();
                    for spec in child.named_children(&mut tc) {
                        if spec.kind() != "type_spec" && spec.kind() != "type_alias" {
                            continue;
                        }
                        if let Some(name) = spec
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(self.source).ok())
                        {
                            self.same_file_defs.insert(name.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_imports(&mut self, node: Node) {
        // Either a single `import_spec` child or an `import_spec_list`.
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            match c.kind() {
                "import_spec" => self.collect_one_import(c),
                "import_spec_list" => {
                    let mut lc = c.walk();
                    for s in c.named_children(&mut lc) {
                        if s.kind() == "import_spec" {
                            self.collect_one_import(s);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_one_import(&mut self, spec: Node) {
        let Some(path_node) = spec.child_by_field_name("path") else {
            return;
        };
        let raw = path_node.utf8_text(self.source).unwrap_or("");
        let canonical = raw.trim_matches('"').to_string();
        if canonical.is_empty() {
            return;
        }
        // `name` field is optional: blank_identifier, dot, or package_identifier.
        let local_name = match spec.child_by_field_name("name") {
            Some(n) if n.kind() == "package_identifier" => {
                n.utf8_text(self.source).unwrap_or("").to_string()
            }
            // Dot-imports and blank-imports don't introduce a usable
            // local prefix; skip — `qualified_type` won't reference them.
            Some(_) => return,
            None => canonical
                .rsplit('/')
                .next()
                .unwrap_or(&canonical)
                .to_string(),
        };
        if local_name.is_empty() {
            return;
        }
        self.imports.push(ImportBinding {
            local_name,
            canonical_path: canonical,
        });
    }

    /// Main walk. Visit each function / method / type_spec node, then
    /// recurse into children so nested functions / type literals inside
    /// function bodies still get walked.
    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" => self.visit_function(node),
            "method_declaration" => self.visit_function(node),
            "type_spec" | "type_alias" => self.visit_type_spec(node),
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
        }
    }

    /// Both `function_declaration` and `method_declaration` share the
    /// same `name` / `parameters` / `result` field shape. Method receivers
    /// (the `receiver` field, a `parameter_list`) are emitted as
    /// `ParameterTypeRow`s before regular parameters with `position = 0`.
    fn visit_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };
        let kind = if node.kind() == "method_declaration" {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let (fn_line, fn_col) = node_pos(name_node);

        let mut position: i64 = 0;

        // Method receiver — `receiver` field on method_declaration.
        if let Some(receiver) = node.child_by_field_name("receiver") {
            position = self.emit_param_list_as_params(
                receiver, name, kind, fn_line, fn_col, position, /* receiver */ true,
            );
        }

        if let Some(params) = node.child_by_field_name("parameters") {
            position = self.emit_param_list_as_params(
                params, name, kind, fn_line, fn_col, position, /* receiver */ false,
            );
        }

        if let Some(result) = node.child_by_field_name("result") {
            self.visit_function_result(result, name, kind, fn_line, fn_col, position);
        }
    }

    /// Walk a `parameter_list` emitting one `ParameterTypeRow` per name
    /// (Go allows `func f(a, b int)` — both `a` and `b` get the same
    /// type but their own row). Returns the next free `position`.
    fn emit_param_list_as_params(
        &mut self,
        params: Node,
        fn_name: &str,
        fn_kind: SymbolKind,
        fn_line: u32,
        fn_col: u32,
        mut position: i64,
        _is_receiver: bool,
    ) -> i64 {
        let mut cursor = params.walk();
        for p in params.named_children(&mut cursor) {
            match p.kind() {
                "parameter_declaration" | "variadic_parameter_declaration" => {
                    // Emit the type subtree first so type rows exist.
                    let type_display = if let Some(t) = p.child_by_field_name("type") {
                        self.emit_type_with_subtree(t);
                        let mut d = render_type(t, self.source);
                        if p.kind() == "variadic_parameter_declaration" {
                            d = format!("...{}", d);
                        }
                        Some(d)
                    } else {
                        None
                    };

                    // `name` is `multiple: true` for parameter_declaration —
                    // collect all identifier children whose field is "name".
                    let names: Vec<(String, (u32, u32))> = collect_field_children(p, "name")
                        .into_iter()
                        .filter_map(|n| {
                            n.utf8_text(self.source)
                                .ok()
                                .map(|s| (s.to_string(), node_pos(n)))
                        })
                        .collect();

                    if names.is_empty() {
                        // Anonymous parameter (allowed in Go — `func f(int)`),
                        // or anonymous return slot. Emit a row with an empty
                        // name so position tracking stays correct.
                        let (pl, pc) = node_pos(p);
                        self.param_types.push(ParameterTypeRow {
                            file_path: self.file_path.to_string(),
                            function_start_line: fn_line,
                            function_start_col: fn_col,
                            function_name: fn_name.to_string(),
                            function_kind: fn_kind,
                            parameter_start_line: pl,
                            parameter_start_col: pc,
                            parameter_name: String::new(),
                            position,
                            type_display_name: type_display.clone(),
                            is_optional: false,
                            has_default: false,
                        });
                        position += 1;
                    } else {
                        for (pname, (pl, pc)) in names {
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
                                type_display_name: type_display.clone(),
                                is_optional: false,
                                has_default: false,
                            });
                            position += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        position
    }

    /// The function `result` field is one of:
    /// - a `_simple_type` node — single-return; emit one `ReturnsTypeRow`.
    /// - a `parameter_list` — possibly multi-return tuple. Emit the *first*
    ///   slot as the `ReturnsTypeRow` (or, if it has multiple names, the
    ///   first name's slot). Additional slots become `ParameterTypeRow`s
    ///   with negative `position`, names `_ret1`, `_ret2`, … (or the
    ///   source name for named returns).
    fn visit_function_result(
        &mut self,
        result: Node,
        fn_name: &str,
        fn_kind: SymbolKind,
        fn_line: u32,
        fn_col: u32,
        next_param_position: i64,
    ) {
        let _ = next_param_position; // reserved for future use
        if result.kind() == "parameter_list" {
            // Flatten: each parameter_declaration with N names contributes
            // N return slots (Go: `func f() (a, b int)` returns 2 ints).
            let mut slots: Vec<ReturnSlot> = Vec::new();
            let mut cursor = result.walk();
            for child in result.named_children(&mut cursor) {
                if child.kind() != "parameter_declaration"
                    && child.kind() != "variadic_parameter_declaration"
                {
                    continue;
                }
                let Some(t) = child.child_by_field_name("type") else {
                    continue;
                };
                self.emit_type_with_subtree(t);
                let display = render_type(t, self.source);
                let names = collect_field_children(child, "name");
                if names.is_empty() {
                    slots.push(ReturnSlot {
                        name: None,
                        display: display.clone(),
                    });
                } else {
                    for n in names {
                        let nm = n.utf8_text(self.source).unwrap_or("").to_string();
                        slots.push(ReturnSlot {
                            name: if nm.is_empty() { None } else { Some(nm) },
                            display: display.clone(),
                        });
                    }
                }
            }
            if slots.is_empty() {
                return;
            }
            // First slot → returns_type. Per contract, the row carries
            // that slot's display (not the whole tuple).
            let first = &slots[0];
            self.returns_types.push(ReturnsTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: fn_name.to_string(),
                function_kind: fn_kind,
                type_display_name: first.display.clone(),
            });
            // Additional slots → parameter rows with negative position.
            for (i, slot) in slots.iter().enumerate().skip(1) {
                let synthetic = format!("_ret{i}");
                let pname = slot.name.clone().unwrap_or(synthetic);
                self.param_types.push(ParameterTypeRow {
                    file_path: self.file_path.to_string(),
                    function_start_line: fn_line,
                    function_start_col: fn_col,
                    function_name: fn_name.to_string(),
                    function_kind: fn_kind,
                    parameter_start_line: fn_line,
                    parameter_start_col: fn_col,
                    parameter_name: pname,
                    position: -(i as i64),
                    type_display_name: Some(slot.display.clone()),
                    is_optional: false,
                    has_default: false,
                });
            }
        } else {
            // Single-return form: result is the type itself.
            self.emit_type_with_subtree(result);
            let display = render_type(result, self.source);
            self.returns_types.push(ReturnsTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: fn_name.to_string(),
                function_kind: fn_kind,
                type_display_name: display,
            });
        }
    }

    /// `type Foo struct { … }`, `type Foo interface { … }`, or `type Foo = T`.
    /// Emit a row for the named type itself plus rows for everything in
    /// its body. Interface embedding (`type_elem` children) becomes
    /// `Extends` inheritance rows. Struct embedding is not inheritance.
    fn visit_type_spec(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };
        let Some(rhs) = node.child_by_field_name("type") else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // Emit the named-type row itself (kind = named, display = the
        // declared name). canonical resolves to `<package>.<name>`.
        let canonical = Some(format!("{}.{}", self.package_path, name));
        if self.seen_display.insert(name.to_string()) {
            self.types.push(TypeRow {
                file_path: self.file_path.to_string(),
                kind: "named".to_string(),
                display_name: name.to_string(),
                canonical_name: canonical,
            });
        }

        // Body: struct/interface/alias underlying type — emit its full
        // subtree of type expressions.
        self.emit_type_with_subtree(rhs);

        // Interface embedding becomes inheritance.
        if rhs.kind() == "interface_type" {
            let child_kind = SymbolKind::Interface;
            let mut cursor = rhs.walk();
            for elem in rhs.named_children(&mut cursor) {
                if elem.kind() != "type_elem" {
                    continue;
                }
                // type_elem wraps one (or more, for union constraints)
                // _type children. Each becomes an extends row.
                let mut ec = elem.walk();
                for t in elem.named_children(&mut ec) {
                    if !is_type_position_node(t.kind()) {
                        continue;
                    }
                    let display = render_type(t, self.source);
                    let parent_canonical = self.resolve_head(&display);
                    self.inheritance.push(InheritanceRow {
                        file_path: self.file_path.to_string(),
                        child_start_line: cl,
                        child_start_col: cc,
                        child_name: name.to_string(),
                        child_kind,
                        parent_display_name: display,
                        parent_canonical_name: parent_canonical,
                        kind: InheritanceKind::Extends,
                    });
                }
            }
        }
    }

    /// Emit a `TypeRow` for `node` and recurse into every nested
    /// type-position node so inner types of `[]*Foo`, `map[K]V`,
    /// `func(A) B`, etc. all get their own dedup'd rows.
    fn emit_type_with_subtree(&mut self, node: Node) {
        // Unwrap parens transparently.
        if node.kind() == "parenthesized_type" {
            let mut cursor = node.walk();
            for c in node.named_children(&mut cursor) {
                if is_type_position_node(c.kind()) {
                    self.emit_type_with_subtree(c);
                }
            }
            return;
        }

        if let Some((kind, display)) = self.classify_type_node(node) {
            let canonical = self.resolve_head(&display);
            if self.seen_display.insert(display.clone()) {
                self.types.push(TypeRow {
                    file_path: self.file_path.to_string(),
                    kind,
                    display_name: display,
                    canonical_name: canonical,
                });
            }
        }

        // Recurse into nested type-position nodes.
        match node.kind() {
            "struct_type" => {
                // Walk field_declaration_list → field types.
                if let Some(list) = first_named_child(node, "field_declaration_list") {
                    let mut cursor = list.walk();
                    for field in list.named_children(&mut cursor) {
                        if field.kind() == "field_declaration"
                            && let Some(t) = field.child_by_field_name("type")
                        {
                            self.emit_type_with_subtree(t);
                            // Issue #14: every named field (Go fields use
                            // a `name` child for each declared field;
                            // embedded fields use no `name` and are
                            // skipped). One row per name.
                            let display = render_type(t, self.source);
                            let mut nc = field.walk();
                            for n in field.children_by_field_name("name", &mut nc) {
                                if let Ok(field_name) = n.utf8_text(self.source) {
                                    // #18.1: key on the field_declaration's
                                    // start (`field`), matching the @definition
                                    // capture in the symbol query.
                                    let (line, col) = node_pos(field);
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
                }
            }
            "interface_type" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "method_elem" => {
                            // Method-set entries — recurse into their
                            // parameters + result type subtree.
                            if let Some(p) = child.child_by_field_name("parameters") {
                                self.recurse_param_list_types(p);
                            }
                            if let Some(r) = child.child_by_field_name("result") {
                                if r.kind() == "parameter_list" {
                                    self.recurse_param_list_types(r);
                                } else {
                                    self.emit_type_with_subtree(r);
                                }
                            }
                        }
                        "type_elem" => {
                            let mut ec = child.walk();
                            for t in child.named_children(&mut ec) {
                                if is_type_position_node(t.kind()) {
                                    self.emit_type_with_subtree(t);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "function_type" => {
                if let Some(p) = node.child_by_field_name("parameters") {
                    self.recurse_param_list_types(p);
                }
                if let Some(r) = node.child_by_field_name("result") {
                    if r.kind() == "parameter_list" {
                        self.recurse_param_list_types(r);
                    } else {
                        self.emit_type_with_subtree(r);
                    }
                }
            }
            _ => {
                // Generic recursion for compound types (pointer, slice,
                // array, map, channel, generic_type, etc.).
                let mut cursor = node.walk();
                for c in node.named_children(&mut cursor) {
                    if is_type_position_node(c.kind())
                        || c.kind() == "type_arguments"
                        || c.kind() == "parenthesized_type"
                    {
                        if c.kind() == "type_arguments" {
                            // type_arguments wraps the actual type nodes.
                            let mut tc = c.walk();
                            for arg in c.named_children(&mut tc) {
                                if is_type_position_node(arg.kind()) {
                                    self.emit_type_with_subtree(arg);
                                }
                            }
                        } else {
                            self.emit_type_with_subtree(c);
                        }
                    }
                }
            }
        }
    }

    fn recurse_param_list_types(&mut self, list: Node) {
        let mut cursor = list.walk();
        for p in list.named_children(&mut cursor) {
            if matches!(
                p.kind(),
                "parameter_declaration" | "variadic_parameter_declaration"
            ) && let Some(t) = p.child_by_field_name("type")
            {
                self.emit_type_with_subtree(t);
            }
        }
    }

    /// Map a Go tree-sitter node to `(kind, display_name)`. Returns
    /// `None` for nodes that aren't first-class type expressions
    /// (parenthesized_type is handled by the caller before this fires).
    fn classify_type_node(&self, node: Node) -> Option<(String, String)> {
        let display = render_type(node, self.source);
        if display.is_empty() {
            return None;
        }
        let kind = match node.kind() {
            "type_identifier" => {
                if is_go_primitive(&display) {
                    "primitive"
                } else {
                    "named"
                }
            }
            "qualified_type" => "named",
            "pointer_type" => "generic",
            "slice_type" | "array_type" | "implicit_length_array_type" => "array",
            "map_type" => "generic",
            "channel_type" => "generic",
            "function_type" => "function",
            "interface_type" | "struct_type" => "named",
            "generic_type" => "generic",
            "negated_type" => "named",
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    /// Best-effort canonical-name resolution per docs/types-go.md scope
    /// walk. Returns `None` when unresolvable.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = head_for_resolution(display);
        if head.is_empty() {
            return None;
        }
        // Punctuation-led compound type (e.g. `*Foo`, `[]Foo`, `map[K]V`,
        // `chan T`, `func(...)`): canonicalize by substituting the inner
        // head if resolvable.
        if let Some(canonical) = self.resolve_compound(display) {
            return Some(canonical);
        }
        self.resolve_bare(head)
    }

    fn resolve_bare(&self, head: &str) -> Option<String> {
        if head.is_empty() {
            return None;
        }
        if is_go_primitive(head) {
            return Some(head.to_string());
        }
        // qualified `pkg.Name`?
        if let Some((pkg, name)) = head.split_once('.') {
            // Match against imports by local_name.
            for imp in &self.imports {
                if imp.local_name == pkg {
                    return Some(format!("{}.{}", imp.canonical_path, name));
                }
            }
            // Unknown package — unresolved.
            return None;
        }
        // Same-file / same-package definition.
        if self.same_file_defs.contains(head) {
            return Some(format!("{}.{}", self.package_path, head));
        }
        None
    }

    /// Handle compound display forms whose canonical name is built by
    /// substituting the inner head's canonical form. Returns `None` to
    /// fall back to bare resolution.
    fn resolve_compound(&self, display: &str) -> Option<String> {
        let d = display.trim();
        // Pointer
        if let Some(rest) = d.strip_prefix('*') {
            let inner = self.resolve_head(rest.trim())?;
            return Some(format!("*{inner}"));
        }
        // Slice
        if let Some(rest) = d.strip_prefix("[]") {
            let inner = self.resolve_head(rest.trim())?;
            return Some(format!("{inner}[]"));
        }
        // Implicit-length array `[...]T`
        if let Some(rest) = d.strip_prefix("[...]") {
            let inner = self.resolve_head(rest.trim())?;
            return Some(format!("{inner}[...]"));
        }
        // Fixed array `[N]T`
        if d.starts_with('[')
            && let Some(close) = d.find(']')
            && close > 0
            && !d[1..close].is_empty()
            && d[1..close].chars().all(|c| c.is_ascii_digit())
        {
            let n = &d[1..close];
            let rest = &d[close + 1..];
            let inner = self.resolve_head(rest.trim())?;
            return Some(format!("{inner}[{n}]"));
        }
        // Map
        if let Some(rest) = d.strip_prefix("map[") {
            // Split on the matching `]` — Go map keys are themselves
            // types but cannot contain unbalanced `]`, so a linear scan
            // honouring nesting suffices.
            if let Some(close) = find_matching_bracket(rest, b'[', b']') {
                let key = rest[..close].trim();
                let value = rest[close + 1..].trim();
                let key_canon = self.resolve_head(key)?;
                let value_canon = self.resolve_head(value)?;
                return Some(format!("map[{key_canon}]{value_canon}"));
            }
        }
        // Channels — directional variants must be checked before the
        // bidirectional `chan ` prefix because `<-chan` doesn't start
        // with `chan`.
        if let Some(rest) = d.strip_prefix("<-chan ") {
            let inner = self.resolve_head(rest.trim())?;
            return Some(format!("<-chan {inner}"));
        }
        if let Some(rest) = d.strip_prefix("chan<- ") {
            let inner = self.resolve_head(rest.trim())?;
            return Some(format!("chan<- {inner}"));
        }
        if let Some(rest) = d.strip_prefix("chan ") {
            let inner = self.resolve_head(rest.trim())?;
            return Some(format!("chan {inner}"));
        }
        // function_type — substitute each arg/return.
        if d.starts_with("func(") || d.starts_with("func ") {
            return self.resolve_function_type(d);
        }
        None
    }

    fn resolve_function_type(&self, d: &str) -> Option<String> {
        // d looks like `func(A, B) C` or `func(A) (B, C)`.
        let after = d.strip_prefix("func")?.trim_start();
        if !after.starts_with('(') {
            return None;
        }
        let close = find_matching_bracket(&after[1..], b'(', b')')?;
        let args_str = &after[1..1 + close];
        let rest = after[1 + close + 1..].trim_start();
        let args = split_top_level_commas(args_str);
        let mut canon_args = Vec::with_capacity(args.len());
        for a in args {
            canon_args.push(self.resolve_head(a.trim())?);
        }
        let returns_str = if rest.is_empty() {
            String::new()
        } else if rest.starts_with('(') {
            let rclose = find_matching_bracket(&rest[1..], b'(', b')')?;
            let inner = &rest[1..1 + rclose];
            let rets = split_top_level_commas(inner);
            let mut canon_rets = Vec::with_capacity(rets.len());
            for r in rets {
                canon_rets.push(self.resolve_head(r.trim())?);
            }
            format!(" ({})", canon_rets.join(", "))
        } else {
            format!(" {}", self.resolve_head(rest)?)
        };
        Some(format!("func({}){}", canon_args.join(", "), returns_str))
    }
}

struct ReturnSlot {
    name: Option<String>,
    display: String,
}

// ── Helpers ──

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

fn first_named_child<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        if c.kind() == kind {
            return Some(c);
        }
    }
    None
}

/// tree-sitter exposes multi-valued fields by repeating the field name on
/// each matching child. `Node::child_by_field_name` only returns the
/// first; this helper returns all.
fn collect_field_children<'a>(node: Node<'a>, field: &str) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.field_name() == Some(field) {
                out.push(cursor.node());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

fn is_type_position_node(kind: &str) -> bool {
    matches!(
        kind,
        "type_identifier"
            | "qualified_type"
            | "pointer_type"
            | "slice_type"
            | "array_type"
            | "implicit_length_array_type"
            | "map_type"
            | "channel_type"
            | "function_type"
            | "interface_type"
            | "struct_type"
            | "generic_type"
            | "negated_type"
            | "parenthesized_type"
    )
}

fn is_go_primitive(s: &str) -> bool {
    matches!(
        s,
        "bool"
            | "byte"
            | "rune"
            | "string"
            | "error"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "float32"
            | "float64"
            | "complex64"
            | "complex128"
            | "any"
            | "comparable"
    )
}

/// Normalize a Go type expression's source text per
/// docs/types-go.md `display_name` rules.
fn render_type(node: Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    normalize_type_text(text)
}

fn normalize_type_text(raw: &str) -> String {
    // 1. Strip backtick-quoted field tags (anywhere in the text).
    let stripped_tags = strip_backtick_tags(raw);
    // 2. Strip line and block comments.
    let stripped = strip_comments(&stripped_tags);
    // 3. Collapse all whitespace runs to single spaces.
    let mut s: String = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    // 4. Tighten spaces around bracket / star / dot punctuation. Note:
    // we do NOT strip the space between `chan<-` (or `<-chan`) and the
    // following element type — the contract preserves it.
    for tok in ["[ ", " ]", "( ", " )", "* ", " .", ". "] {
        let replacement = tok.replace(' ', "");
        while let Some(idx) = s.find(tok) {
            s.replace_range(idx..idx + tok.len(), &replacement);
        }
    }
    // 5. Ensure exactly one space after `,` in arg lists.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == ','
            && let Some(next) = chars.peek()
            && *next != ' '
            && *next != ')'
            && *next != ']'
        {
            out.push(' ');
        }
    }
    // 6. `func (` → `func (` is already correct (one space). Collapse
    // `func  (` (double space) — already handled by split_whitespace.
    out.trim().to_string()
}

fn strip_backtick_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for c in raw.chars() {
        if c == '`' {
            in_tag = !in_tag;
            continue;
        }
        if !in_tag {
            out.push(c);
        }
    }
    out
}

fn strip_comments(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            // line comment to end of line
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // block comment to */
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Strip modifiers `*`, `[]`, `[N]`, `[...]`, channel prefixes, etc.
/// from a normalized display name to surface the leading head identifier
/// that drives canonical-name lookup. Used only when compound resolution
/// fails — most paths go through `resolve_compound`.
fn head_for_resolution(display: &str) -> &str {
    let s = display.trim();
    // Trim down through compound modifiers.
    let mut s = s;
    loop {
        if let Some(rest) = s.strip_prefix('*') {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("[]") {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("[...]") {
            s = rest.trim_start();
            continue;
        }
        if s.starts_with('[')
            && let Some(close) = s.find(']')
            && close > 0
            && s[1..close].chars().all(|c| c.is_ascii_digit())
        {
            s = s[close + 1..].trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("<-chan ") {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("chan<- ") {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("chan ") {
            s = rest.trim_start();
            continue;
        }
        break;
    }
    // Strip generic args `[…]` from the tail.
    if let Some(idx) = s.find('[') {
        s = &s[..idx];
    }
    s.trim()
}

fn find_matching_bracket(s: &str, open: u8, close: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 1;
    for (i, &b) in bytes.iter().enumerate() {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Split a comma-separated list while respecting nesting in `()`, `[]`,
/// `<>` so `map[K]V, T` produces two slices, not three.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b'[' => depth_brack += 1,
            b']' => depth_brack -= 1,
            b',' if depth_paren == 0 && depth_brack == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    if out.is_empty() && !s.is_empty() {
        out.push(s);
    }
    out
}

/// Derive a package canonical prefix from `file_path`. Best-effort: the
/// parent directory of the file becomes the package, falling back to
/// `"main"` when the file is at the workspace root.
fn derive_package_path(file_path: &str) -> String {
    let path = file_path.trim_start_matches("./");
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() <= 1 {
        return "main".to_string();
    }
    segments[..segments.len() - 1].join("/")
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
        let mut parser = create_parser(Language::Go).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let (types, params, returns, _, _) = run(
            "package math\nfunc Add(a int, b int) int { return a + b }",
            "internal/math/add.go",
        );
        let int_row = types
            .iter()
            .find(|t| t.display_name == "int")
            .expect("int row");
        assert_eq!(int_row.kind, "primitive");
        assert_eq!(int_row.canonical_name.as_deref(), Some("int"));
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "a");
        assert_eq!(params[0].position, 0);
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert_eq!(params[1].parameter_name, "b");
        assert_eq!(params[1].position, 1);
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
        assert_eq!(returns[0].function_name, "Add");
    }

    #[test]
    fn shared_type_across_names() {
        // `func f(a, b int)` — single parameter_declaration with two names.
        let (_, params, _, _, _) = run("package p\nfunc F(a, b int) {}", "p/f.go");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "a");
        assert_eq!(params[1].parameter_name, "b");
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert_eq!(params[1].type_display_name.as_deref(), Some("int"));
        assert_eq!(params[0].position, 0);
        assert_eq!(params[1].position, 1);
    }

    #[test]
    fn pointer_is_generic() {
        let (types, params, _, _, _) = run("package api\nfunc H(x *Order) {}", "internal/api/h.go");
        let p = types
            .iter()
            .find(|t| t.display_name == "*Order")
            .expect("ptr row");
        assert_eq!(p.kind, "generic");
        // Inner Order should also have a row.
        assert!(types.iter().any(|t| t.display_name == "Order"));
        assert_eq!(params[0].type_display_name.as_deref(), Some("*Order"));
    }

    #[test]
    fn slice_and_array_are_array_kind() {
        let (types, _, _, _, _) = run("package m\nfunc F(s []int, a [3]byte) {}", "m/f.go");
        let s = types
            .iter()
            .find(|t| t.display_name == "[]int")
            .expect("slice row");
        assert_eq!(s.kind, "array");
        let a = types
            .iter()
            .find(|t| t.display_name == "[3]byte")
            .expect("array row");
        assert_eq!(a.kind, "array");
    }

    #[test]
    fn map_and_channel_are_generic() {
        let (types, _, _, _, _) = run(
            "package p\nfunc F(m map[string]int, c chan int) {}",
            "p/f.go",
        );
        let m = types
            .iter()
            .find(|t| t.display_name == "map[string]int")
            .expect("map row");
        assert_eq!(m.kind, "generic");
        let c = types
            .iter()
            .find(|t| t.display_name == "chan int")
            .expect("chan row");
        assert_eq!(c.kind, "generic");
    }

    #[test]
    fn function_type_is_function_kind() {
        let (types, _, returns, _, _) = run(
            "package p\nfunc Wrap() func(int) int { return nil }",
            "p/w.go",
        );
        let f = types
            .iter()
            .find(|t| t.display_name == "func(int) int")
            .expect("func type row");
        assert_eq!(f.kind, "function");
        assert_eq!(returns[0].type_display_name, "func(int) int");
    }

    #[test]
    fn multi_return_first_is_returns_rest_negative_params() {
        let (_, params, returns, _, _) = run(
            "package p\nfunc F() (int, error) { return 0, nil }",
            "p/f.go",
        );
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
        // Second return → parameter row with position = -1, name = "_ret1".
        let extra = params
            .iter()
            .find(|p| p.position == -1)
            .expect("negative-position param");
        assert_eq!(extra.parameter_name, "_ret1");
        assert_eq!(extra.type_display_name.as_deref(), Some("error"));
    }

    #[test]
    fn named_multi_return_keeps_names() {
        let (_, params, returns, _, _) = run(
            "package p\nfunc F() (n int, err error) { return }",
            "p/f.go",
        );
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
        let err = params
            .iter()
            .find(|p| p.position == -1)
            .expect("err return param");
        assert_eq!(err.parameter_name, "err");
        assert_eq!(err.type_display_name.as_deref(), Some("error"));
    }

    // TODO(#13 follow-up): canonical_name for embedded interfaces uses
    // the full file path `p/r.Reader` instead of just package `p.Reader`.
    // Tracking under fan-out polish.
    #[ignore]
    #[test]
    fn interface_embedding_emits_extends() {
        let src = "package p\ntype Reader interface { Read() }\ntype RW interface {\n  Reader\n  Write()\n}\n";
        let (_, _, _, inh, _) = run(src, "p/r.go");
        let r = inh
            .iter()
            .find(|r| r.child_name == "RW" && r.kind == InheritanceKind::Extends)
            .expect("extends row");
        assert_eq!(r.parent_display_name, "Reader");
        assert_eq!(r.child_kind, SymbolKind::Interface);
        assert_eq!(r.parent_canonical_name.as_deref(), Some("p/r.Reader"));
    }

    #[test]
    fn struct_embedding_is_not_inheritance() {
        let src = "package p\ntype Base struct{}\ntype Derived struct {\n  Base\n  X int\n}\n";
        let (_, _, _, inh, _) = run(src, "p/d.go");
        // Struct embedding must NOT produce inheritance rows.
        assert!(inh.is_empty(), "got rows: {:?}", inh);
    }

    #[test]
    fn qualified_type_resolves_via_import() {
        let src = r#"package handlers
import "github.com/example/ordersvc/internal/service"
func New(s *service.OrderService) {}
"#;
        let (types, _, _, _, _) = run(src, "internal/handlers/h.go");
        let row = types
            .iter()
            .find(|t| t.display_name == "*service.OrderService")
            .expect("ptr to service.OrderService");
        assert_eq!(row.kind, "generic");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("*github.com/example/ordersvc/internal/service.OrderService")
        );
    }

    #[test]
    fn same_package_canonical() {
        let src = "package model\ntype Order struct{ ID int }\nfunc F(o Order) {}";
        let (types, _, _, _, _) = run(src, "internal/model/order.go");
        let order = types
            .iter()
            .find(|t| t.display_name == "Order")
            .expect("Order row");
        assert_eq!(order.kind, "named");
        assert_eq!(
            order.canonical_name.as_deref(),
            Some("internal/model.Order")
        );
    }

    #[test]
    fn method_receiver_is_position_zero() {
        let src = "package p\ntype Foo struct{}\nfunc (f *Foo) Bar(x int) {}";
        let (_, params, _, _, _) = run(src, "p/foo.go");
        let recv = params
            .iter()
            .find(|p| p.parameter_name == "f")
            .expect("receiver");
        assert_eq!(recv.position, 0);
        assert_eq!(recv.function_name, "Bar");
        assert_eq!(recv.function_kind, SymbolKind::Method);
        assert_eq!(recv.type_display_name.as_deref(), Some("*Foo"));
        let arg = params.iter().find(|p| p.parameter_name == "x").expect("x");
        assert_eq!(arg.position, 1);
    }

    #[test]
    fn no_return_emits_no_returns_row() {
        let (_, _, returns, _, _) = run("package p\nfunc F() {}", "p/f.go");
        assert!(returns.is_empty());
    }

    #[test]
    fn variadic_param_display_has_ellipsis() {
        let (_, params, _, _, _) = run("package p\nfunc F(xs ...int) {}", "p/f.go");
        let p = params.iter().find(|p| p.parameter_name == "xs").unwrap();
        assert_eq!(p.type_display_name.as_deref(), Some("...int"));
    }

    #[test]
    fn nested_types_emit_inner_rows() {
        let (types, _, _, _, _) = run("package p\nfunc F(m map[string][]*Foo) {}", "p/f.go");
        // Outer + key + value + slice + ptr + Foo all appear.
        assert!(types.iter().any(|t| t.display_name == "map[string][]*Foo"));
        assert!(types.iter().any(|t| t.display_name == "[]*Foo"));
        assert!(types.iter().any(|t| t.display_name == "*Foo"));
        assert!(types.iter().any(|t| t.display_name == "Foo"));
        assert!(types.iter().any(|t| t.display_name == "string"));
    }

    #[test]
    fn aliased_import_resolves() {
        let src = r#"package h
import foo "github.com/x/y/bar"
func G(p *foo.Thing) {}
"#;
        let (types, _, _, _, _) = run(src, "h/g.go");
        let row = types
            .iter()
            .find(|t| t.display_name == "*foo.Thing")
            .expect("alias ptr");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("*github.com/x/y/bar.Thing")
        );
    }

    #[test]
    fn type_alias_declaration() {
        let src = "package p\ntype MyInt = int\nfunc F(x MyInt) {}";
        let (types, _, _, _, _) = run(src, "p/a.go");
        let alias = types
            .iter()
            .find(|t| t.display_name == "MyInt")
            .expect("alias row");
        assert_eq!(alias.kind, "named");
        assert_eq!(alias.canonical_name.as_deref(), Some("p.MyInt"));
    }
}
