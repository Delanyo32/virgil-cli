//! Issue #13 Rust pilot — type-expression / signature / inheritance
//! extractor. Per ADR-0003 (Level 3): full kind decomposition +
//! canonical_name resolution. Contract: docs/types-rust.md.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::models::{
    ExtractedTypes, FieldTypeRow, InheritanceKind, InheritanceRow, ParameterTypeRow,
    ReturnsTypeRow, SymbolKind, TypeRow,
};

pub fn extract_types(tree: &Tree, source: &[u8], file_path: &str) -> ExtractedTypes {
    let mut ctx = Ctx::new(file_path, source);
    ctx.collect_file_level(tree.root_node());
    ctx.walk(tree.root_node(), false);
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
    /// Use statements at file scope, parsed into `(local_name, canonical_path)` pairs.
    use_bindings: Vec<UseBinding>,
    /// Same-file definitions that can resolve a bare name to a
    /// `crate::<module>::<name>` canonical form.
    same_file_defs: HashSet<String>,
    /// Derived `crate::<module>` prefix for this file, per the
    /// `<crate>::<module-path>` rule in docs/types-rust.md.
    crate_module_path: String,
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
            same_file_defs: HashSet::new(),
            crate_module_path: derive_crate_module_path(file_path),
        }
    }

    fn finish(self) -> ExtractedTypes {
        (
            self.types,
            self.param_types,
            self.returns_types,
            self.inheritance,
            self.field_types,
        )
    }

    /// Pre-pass: collect file-scope `use` bindings + same-file type
    /// definitions so the main walk can resolve `canonical_name`.
    fn collect_file_level(&mut self, root: Node) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "use_declaration" => {
                    collect_use_bindings(child, self.source, &mut self.use_bindings)
                }
                "struct_item" | "enum_item" | "trait_item" | "type_item" | "union_item" => {
                    if let Some(name) = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                    {
                        self.same_file_defs.insert(name.to_string());
                    }
                }
                "mod_item" => {
                    // Inline module: its `struct foo {}` etc. are still
                    // same-file from a canonical-name perspective for
                    // file-relative resolution.
                    if let Some(body) = child.child_by_field_name("body") {
                        self.collect_file_level(body);
                    }
                }
                _ => {}
            }
        }
    }

    /// Main walk. `inside_test` is set when we descend into a
    /// `#[cfg(test)]` module — not used for filtering here, just
    /// reserved for future test-aware decisions.
    fn walk(&mut self, node: Node, _inside_test: bool) {
        match node.kind() {
            "function_item" => self.visit_function(node),
            "struct_item" => self.visit_struct(node),
            "union_item" => self.visit_union(node),
            "enum_item" => self.visit_enum(node),
            "trait_item" => self.visit_trait(node),
            "type_item" => self.visit_type_alias(node),
            "impl_item" => self.visit_impl(node),
            "let_declaration" => self.visit_let(node),
            _ => {}
        }

        // Recurse — we still visit children even after handling the
        // current node because nested functions / impls inside a
        // function body are valid Rust.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, false);
        }
    }

    /// `fn foo<T>(a: A, b: B) -> R { ... }` and `impl X { fn bar(&self) -> Y { ... } }`.
    fn visit_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };
        let kind = if is_inside_impl_or_trait(node) {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let (fn_line, fn_col) = (
            name_node.start_position().row as u32 + 1,
            name_node.start_position().column as u32,
        );

        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, name, kind, fn_line, fn_col);
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            let display = render_type(ret, self.source);
            self.emit_type_with_subtree(ret);
            self.returns_types.push(ReturnsTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: name.to_string(),
                function_kind: kind,
                type_display_name: display,
            });
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
                "self_parameter" => {
                    // `self`, `&self`, `&mut self` — emit a Parameter row
                    // with name="self" and untyped (type_display_name=None
                    // since the receiver type is the enclosing impl's
                    // self type, not written on the parameter).
                    let (pl, pc) = node_pos(p);
                    self.param_types.push(ParameterTypeRow {
                        file_path: self.file_path.to_string(),
                        function_start_line: fn_line,
                        function_start_col: fn_col,
                        function_name: fn_name.to_string(),
                        function_kind: fn_kind,
                        parameter_start_line: pl,
                        parameter_start_col: pc,
                        parameter_name: "self".into(),
                        position,
                        type_display_name: None,
                        is_optional: false,
                        has_default: false,
                    });
                    position += 1;
                }
                "parameter" => {
                    let pat_name = p
                        .child_by_field_name("pattern")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let (pl, pc) = node_pos(p.child_by_field_name("pattern").unwrap_or(p));
                    let type_display = if let Some(t) = p.child_by_field_name("type") {
                        self.emit_type_with_subtree(t);
                        Some(render_type(t, self.source))
                    } else {
                        None
                    };
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
                        is_optional: false,
                        has_default: false,
                    });
                    position += 1;
                }
                _ => {}
            }
        }
    }

    fn visit_struct(&mut self, node: Node) {
        if let Some(body) = node.child_by_field_name("body") {
            self.emit_field_types(body);
        }
    }

    fn visit_union(&mut self, node: Node) {
        if let Some(body) = node.child_by_field_name("body") {
            self.emit_field_types(body);
        }
    }

    fn visit_enum(&mut self, node: Node) {
        // Enum variants may carry tuple- or struct-shaped fields; emit
        // their type rows so the `type` table sees them.
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for v in body.named_children(&mut cursor) {
                if v.kind() != "enum_variant" {
                    continue;
                }
                if let Some(b) = v.child_by_field_name("body") {
                    self.emit_field_types(b);
                }
            }
        }
    }

    /// `trait Sub: Super1 + Super2 + 'a { ... }` — the bound list becomes
    /// `extends` rows from `Sub` to each super-trait.
    fn visit_trait(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);
        if let Some(bounds) = node.child_by_field_name("bounds") {
            self.collect_bound_inheritance(
                bounds,
                child_name,
                SymbolKind::Trait,
                cl,
                cc,
                InheritanceKind::Extends,
            );
        }
    }

    fn visit_type_alias(&mut self, node: Node) {
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
        }
    }

    /// `impl Trait for Type { ... }` → implements(Type, Trait).
    /// `impl Type { ... }` → no edge.
    fn visit_impl(&mut self, node: Node) {
        let Some(type_node) = node.child_by_field_name("type") else {
            return;
        };
        let Ok(type_name) = type_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(type_node);
        if let Some(trait_node) = node.child_by_field_name("trait") {
            let display = render_type(trait_node, self.source);
            self.emit_type_with_subtree(trait_node);
            let canonical = self.resolve_head(&display);
            // The "child" here is the type; the "interface" is the trait.
            // We point at the unqualified head of `type_node` as the
            // child name — symbol resolution joins to the workspace
            // symbol by name later.
            let child_head = extract_head_segment(type_name);
            self.inheritance.push(InheritanceRow {
                file_path: self.file_path.to_string(),
                child_start_line: cl,
                child_start_col: cc,
                child_name: child_head.to_string(),
                child_kind: SymbolKind::Struct, // placeholder; resolver matches by name
                parent_display_name: display,
                parent_canonical_name: canonical,
                kind: InheritanceKind::Implements,
            });
        }
    }

    fn visit_let(&mut self, node: Node) {
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
        }
    }

    fn emit_field_types(&mut self, body: Node) {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            if child.kind() != "field_declaration" {
                continue;
            }
            let Some(t) = child.child_by_field_name("type") else {
                continue;
            };
            self.emit_type_with_subtree(t);
            // Issue #14 + #18.1: emit FieldTypeRow keyed on the
            // *field_declaration* start position so the synthesized
            // symbol_id matches the Symbol row the symbol query
            // produces (which captures @definition = field_declaration).
            if let Some(name_node) = child.child_by_field_name("name")
                && let Ok(field_name) = name_node.utf8_text(self.source)
            {
                let (line, col) = node_pos(child);
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
    }

    fn collect_bound_inheritance(
        &mut self,
        bounds: Node,
        child_name: &str,
        child_kind: SymbolKind,
        cl: u32,
        cc: u32,
        kind: InheritanceKind,
    ) {
        let mut cursor = bounds.walk();
        for b in bounds.named_children(&mut cursor) {
            match b.kind() {
                "lifetime" | "removed_trait_bound" => continue,
                _ => {}
            }
            // Each bound participates in the parent display_name; emit
            // its full type subtree and a row pointing at it.
            self.emit_type_with_subtree(b);
            let display = render_type(b, self.source);
            let canonical = self.resolve_head(&display);
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

    /// Emit a TypeRow for `node` and every meaningful sub-type expression
    /// nested inside it. Per docs/types-rust.md `display_name`
    /// construction.
    fn emit_type_with_subtree(&mut self, node: Node) {
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
        // Recurse into children so nested types (e.g. `Vec<HashMap<K,V>>`
        // emits rows for the inner HashMap, K, V too) land their own
        // rows.
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if !is_type_position_node(c.kind()) {
                continue;
            }
            self.emit_type_with_subtree(c);
        }
    }

    /// Map a tree-sitter node to its schema `kind` + the rendered
    /// `display_name`. Returns `None` for non-type nodes (e.g. lifetimes
    /// appearing standalone, which fold into the parent).
    fn classify_type_node(&self, node: Node) -> Option<(String, String)> {
        let display = render_type(node, self.source);
        if display.is_empty() {
            return None;
        }
        let kind = match node.kind() {
            "primitive_type" | "unit_type" | "never_type" => "primitive",
            "type_identifier" => {
                // Could refer to a generic parameter (no canonical) or a
                // named type; either way, schema kind is `named`.
                "named"
            }
            "scoped_type_identifier" => "named",
            "generic_type" => "generic",
            "reference_type" => "generic",
            "pointer_type" => "generic",
            "tuple_type" => "tuple",
            "array_type" | "slice_type" => "array",
            "function_type" => "function",
            "dynamic_type" | "abstract_type" => {
                if count_trait_bounds(node) >= 2 {
                    "intersection"
                } else {
                    "named"
                }
            }
            "bounded_type" => {
                if count_trait_bounds(node) >= 2 {
                    "intersection"
                } else {
                    "named"
                }
            }
            "macro_invocation" => "named",
            "lifetime" | "removed_trait_bound" => return None,
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    /// Resolve `display` to a canonical name via the scope-walk order in
    /// docs/types-rust.md. Returns `None` when unresolvable.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = strip_modifiers(display);
        let head = strip_generic_args(head);
        let head = head.trim();
        if head.is_empty() {
            return None;
        }
        // Special placeholder Self → no canonical resolution.
        if head == "Self" || head == "_" {
            return None;
        }
        // 1. Primitives → primitive name.
        if is_primitive(head) {
            return Some(head.to_string());
        }
        // 2. crate:: / self:: / super:: → keep verbatim; downstream
        //    queries can resolve through `imports` if needed.
        if head.starts_with("crate::") || head.starts_with("self::") || head.starts_with("super::")
        {
            return Some(head.to_string());
        }
        // 3. std:: / core:: / alloc:: → preserved verbatim.
        if head.starts_with("std::") || head.starts_with("core::") || head.starts_with("alloc::") {
            return Some(head.to_string());
        }
        // 4. `use` alias match — try the unqualified leaf and any
        //    `path::like::this` prefix's local binding.
        let first_segment = head.split("::").next().unwrap_or(head);
        for u in &self.use_bindings {
            if u.local_name == first_segment {
                // Replace the matched prefix with the canonical path.
                if head == first_segment {
                    return Some(u.canonical_path.clone());
                }
                let rest = &head[first_segment.len()..]; // includes leading ::
                return Some(format!("{}{}", u.canonical_path, rest));
            }
        }
        // 5. Same-file definition → crate-relative path.
        if let Some(name) = head.split("::").next()
            && self.same_file_defs.contains(name)
            && head == name
        {
            return Some(format!("{}::{}", self.crate_module_path, name));
        }
        // 6. Standard prelude common names — treat as std even without a
        //    use statement (covers `String`, `Vec`, `Option`, `Result`,
        //    `Box`, etc., per the contract's Example 3 / 6 worked cases).
        if let Some(p) = prelude_canonical(head) {
            return Some(p);
        }
        None
    }
}

/// `src/foo/bar.rs` → `crate::foo::bar`; `src/lib.rs` or `src/main.rs` →
/// `crate`; `src/foo/mod.rs` → `crate::foo`. Files outside `src/` get
/// `crate::<segments>` from their workspace-relative path.
fn derive_crate_module_path(file_path: &str) -> String {
    let path = file_path.trim_start_matches("./");
    let mut segments: Vec<&str> = path.split('/').collect();
    if let Some(first) = segments.first()
        && (*first == "src")
    {
        segments.remove(0);
    }
    if let Some(last) = segments.last_mut() {
        *last = last.strip_suffix(".rs").unwrap_or(last);
    }
    if let Some(last) = segments.last()
        && (*last == "mod" || *last == "lib" || *last == "main")
    {
        segments.pop();
    }
    if segments.is_empty() {
        "crate".to_string()
    } else {
        format!("crate::{}", segments.join("::"))
    }
}

fn collect_use_bindings(node: Node, source: &[u8], out: &mut Vec<UseBinding>) {
    let Some(arg) = node.child_by_field_name("argument") else {
        return;
    };
    parse_use_tree(arg, source, "", out);
}

/// Recursively flatten a `use_declaration` argument into
/// `(local_name, canonical_path)` pairs. Handles:
///   use a::b::c;            → ("c", "a::b::c")
///   use a::b as d;          → ("d", "a::b")
///   use a::{b, c as d, e::*};
fn parse_use_tree(node: Node, source: &[u8], prefix: &str, out: &mut Vec<UseBinding>) {
    match node.kind() {
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                parse_use_tree(child, source, prefix, out);
            }
        }
        "scoped_use_list" => {
            let Some(path) = node.child_by_field_name("path") else {
                return;
            };
            let Some(list) = node.child_by_field_name("list") else {
                return;
            };
            let path_text = path.utf8_text(source).unwrap_or("").trim().to_string();
            let new_prefix = if prefix.is_empty() {
                path_text
            } else {
                format!("{prefix}::{path_text}")
            };
            parse_use_tree(list, source, &new_prefix, out);
        }
        "use_as_clause" => {
            // `use a::b::c as d;`
            let Some(path) = node.child_by_field_name("path") else {
                return;
            };
            let Some(alias) = node.child_by_field_name("alias") else {
                return;
            };
            let path_text = path.utf8_text(source).unwrap_or("").trim().to_string();
            let canonical = if prefix.is_empty() {
                path_text
            } else {
                format!("{prefix}::{path_text}")
            };
            let alias_text = alias.utf8_text(source).unwrap_or("").trim().to_string();
            if !alias_text.is_empty() {
                out.push(UseBinding {
                    local_name: alias_text,
                    canonical_path: canonical,
                });
            }
        }
        "use_wildcard" => {
            // `use a::*;` — record one binding per "*" so the resolver
            // doesn't try to match it. Skipping for now; downstream
            // queries can join `imports` for transitive resolution.
        }
        "scoped_identifier" | "identifier" => {
            let text = node.utf8_text(source).unwrap_or("").trim().to_string();
            if text.is_empty() {
                return;
            }
            let canonical = if prefix.is_empty() {
                text.clone()
            } else {
                format!("{prefix}::{text}")
            };
            let local_name = canonical
                .rsplit("::")
                .next()
                .unwrap_or(&canonical)
                .to_string();
            out.push(UseBinding {
                local_name,
                canonical_path: canonical,
            });
        }
        _ => {}
    }
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

fn is_inside_impl_or_trait(node: Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "impl_item" | "trait_item" => return true,
            "declaration_list" => {
                current = parent.parent();
                continue;
            }
            _ => return false,
        }
    }
    false
}

fn is_type_position_node(kind: &str) -> bool {
    matches!(
        kind,
        "primitive_type"
            | "unit_type"
            | "never_type"
            | "type_identifier"
            | "scoped_type_identifier"
            | "generic_type"
            | "reference_type"
            | "pointer_type"
            | "tuple_type"
            | "array_type"
            | "slice_type"
            | "function_type"
            | "dynamic_type"
            | "abstract_type"
            | "bounded_type"
            | "macro_invocation"
            | "type_arguments"
    )
}

fn count_trait_bounds(node: Node) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        match c.kind() {
            "type_identifier" | "scoped_type_identifier" | "generic_type" | "function_type" => {
                count += 1;
            }
            "lifetime" | "removed_trait_bound" => {}
            _ => {}
        }
    }
    count
}

/// Render a type node into the normalized `display_name` form per
/// docs/types-rust.md "display_name construction" rules. Rules:
///   - Collapse runs of ASCII whitespace to single spaces, trim.
///   - One space after `,` or `;` separating list elements; no internal
///     whitespace around `<`, `>`, `(`, `)`, `[`, `]`, `::`, `&`, `*`.
///   - Preserve `dyn` / `impl` / `unsafe` / `extern "C"` qualifiers and
///     lifetimes inside angle brackets / references.
fn render_type(node: Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    normalize_type_text(text)
}

fn normalize_type_text(raw: &str) -> String {
    // First collapse whitespace runs.
    let mut s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Remove spaces around punctuation tokens that carry no internal
    // whitespace.
    for tok in ["< ", " >", "( ", " )", "[ ", " ]", " ::", ":: ", "& ", "* "] {
        let replacement = tok.replace(' ', "");
        while let Some(idx) = s.find(tok) {
            s.replace_range(idx..idx + tok.len(), &replacement);
        }
    }
    // Re-insert space after `,` if missing (handles `,T`).
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if (c == ',' || c == ';')
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

fn strip_modifiers(display: &str) -> &str {
    let mut s = display.trim();
    // Strip reference / pointer prefixes iteratively. `&mut T`, `&'a T`,
    // `*const T`, `*mut T`.
    loop {
        let trimmed = s;
        if let Some(rest) = trimmed.strip_prefix('&') {
            // Skip optional `mut ` or lifetime `'a `.
            let r = rest.trim_start();
            let r = r.strip_prefix("mut ").unwrap_or(r).trim_start();
            let r = if r.starts_with('\'') {
                // skip lifetime token until whitespace
                let tail = r.split_once(' ').map(|(_, b)| b).unwrap_or("");
                tail.trim_start()
            } else {
                r
            };
            s = r;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('*') {
            let r = rest.trim_start();
            let r = r
                .strip_prefix("const ")
                .or_else(|| r.strip_prefix("mut "))
                .unwrap_or(r)
                .trim_start();
            s = r;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("dyn ") {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("impl ") {
            s = rest.trim_start();
            continue;
        }
        break;
    }
    s
}

fn strip_generic_args(s: &str) -> &str {
    match s.find('<') {
        Some(idx) => &s[..idx],
        None => s,
    }
}

fn extract_head_segment(s: &str) -> &str {
    let s = strip_modifiers(s);
    let s = strip_generic_args(s);
    s.split("::").last().unwrap_or(s).trim()
}

fn is_primitive(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "()"
            | "!"
    )
}

/// Names that resolve via the standard Rust prelude even without an
/// explicit `use`. Mirrors the canonical paths in docs/types-rust.md
/// worked examples (`String` → `alloc::string::String`,
/// `Result` → `std::result::Result`, etc.).
fn prelude_canonical(name: &str) -> Option<String> {
    let p = match name {
        "String" => "alloc::string::String",
        "Vec" => "alloc::vec::Vec",
        "Box" => "alloc::boxed::Box",
        "Rc" => "alloc::rc::Rc",
        "Arc" => "alloc::sync::Arc",
        "Option" => "core::option::Option",
        "Result" => "core::result::Result",
        "Iterator" => "core::iter::Iterator",
        "FnOnce" => "core::ops::FnOnce",
        "FnMut" => "core::ops::FnMut",
        "Fn" => "core::ops::Fn",
        "Send" => "core::marker::Send",
        "Sync" => "core::marker::Sync",
        "Copy" => "core::marker::Copy",
        "Clone" => "core::clone::Clone",
        "Sized" => "core::marker::Sized",
        "Drop" => "core::ops::Drop",
        "Default" => "core::default::Default",
        "PartialEq" => "core::cmp::PartialEq",
        "Eq" => "core::cmp::Eq",
        "PartialOrd" => "core::cmp::PartialOrd",
        "Ord" => "core::cmp::Ord",
        "Hash" => "core::hash::Hash",
        "Debug" => "core::fmt::Debug",
        "Display" => "core::fmt::Display",
        "ToString" => "alloc::string::ToString",
        "From" => "core::convert::From",
        "Into" => "core::convert::Into",
        "TryFrom" => "core::convert::TryFrom",
        "TryInto" => "core::convert::TryInto",
        "AsRef" => "core::convert::AsRef",
        "AsMut" => "core::convert::AsMut",
        _ => return None,
    };
    Some(p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;

    fn run(source: &str, path: &str) -> ExtractedTypes {
        let mut parser = create_parser(Language::Rust).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let (types, params, returns, _, _) =
            run("fn add(a: i32, b: i32) -> i32 { a + b }", "src/lib.rs");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "i32" && t.kind == "primitive")
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].type_display_name.as_deref(), Some("i32"));
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "i32");
    }

    #[test]
    fn reference_param() {
        let (types, params, _, _, _) = run("fn f(s: &str) {}", "src/lib.rs");
        // &str → kind=generic; inner str → primitive
        let outer = types
            .iter()
            .find(|t| t.display_name == "&str")
            .expect("outer");
        assert_eq!(outer.kind, "generic");
        let inner = types
            .iter()
            .find(|t| t.display_name == "str")
            .expect("inner");
        assert_eq!(inner.kind, "primitive");
        assert_eq!(params[0].type_display_name.as_deref(), Some("&str"));
    }

    #[test]
    fn generic_return() {
        let (types, _, returns, _, _) =
            run("fn f() -> Result<bool, String> { Ok(true) }", "src/lib.rs");
        let outer = types
            .iter()
            .find(|t| t.display_name == "Result<bool, String>")
            .expect("outer");
        assert_eq!(outer.kind, "generic");
        assert_eq!(
            outer.canonical_name.as_deref(),
            Some("core::result::Result")
        );
        assert_eq!(returns[0].type_display_name, "Result<bool, String>");
    }

    #[test]
    fn tuple_and_array() {
        let (types, _, _, _, _) = run(
            "pub fn f(r: &[(String, bool, u64)]) {}",
            "src/cli/output.rs",
        );
        assert!(types.iter().any(|t| t.kind == "tuple"));
        assert!(types.iter().any(|t| t.kind == "array"));
    }

    #[test]
    fn same_file_canonical() {
        let src = "pub struct ContentHash { value: u64 }\nfn f(c: ContentHash) {}";
        let (types, _, _, _, _) = run(src, "src/utils/hash.rs");
        let row = types
            .iter()
            .find(|t| t.display_name == "ContentHash")
            .expect("row");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("crate::utils::hash::ContentHash")
        );
    }

    #[test]
    fn use_alias_resolution() {
        let src = "use std::collections::HashMap as Map;\nfn f(m: Map<String, u8>) {}";
        let (types, _, _, _, _) = run(src, "src/lib.rs");
        let row = types
            .iter()
            .find(|t| t.display_name.starts_with("Map<"))
            .expect("row");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("std::collections::HashMap")
        );
    }

    #[test]
    fn trait_extends() {
        let src = "trait Sub: Super + Send {}\ntrait Super {}";
        let (_, _, _, inh, _) = run(src, "src/lib.rs");
        let supers: Vec<&str> = inh
            .iter()
            .filter(|r| r.child_name == "Sub" && r.kind == InheritanceKind::Extends)
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert!(supers.contains(&"Super"), "missing Super in {:?}", supers);
        assert!(supers.contains(&"Send"), "missing Send in {:?}", supers);
    }

    #[test]
    fn impl_implements() {
        let src = "struct Foo;\nimpl Bar for Foo {}";
        let (_, _, _, inh, _) = run(src, "src/lib.rs");
        let row = inh
            .iter()
            .find(|r| r.kind == InheritanceKind::Implements)
            .expect("implements row");
        assert_eq!(row.child_name, "Foo");
        assert_eq!(row.parent_display_name, "Bar");
    }

    #[test]
    fn struct_fields_emit_field_types() {
        let src = "pub struct Cache { entries: u64, name: String }";
        let (_, _, _, _, fields) = run(src, "src/core/cache.rs");
        let names: Vec<&str> = fields.iter().map(|f| f.field_name.as_str()).collect();
        assert!(names.contains(&"entries"), "got {:?}", names);
        assert!(names.contains(&"name"), "got {:?}", names);
        let n = fields.iter().find(|f| f.field_name == "entries").unwrap();
        assert_eq!(n.type_display_name, "u64");
        assert_eq!(n.field_kind, SymbolKind::Field);
    }

    #[test]
    fn self_parameter_emitted() {
        let src = "struct S;\nimpl S { fn m(&self, x: i32) {} }";
        let (_, params, _, _, _) = run(src, "src/lib.rs");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "self");
        assert_eq!(params[0].position, 0);
        assert_eq!(params[1].parameter_name, "x");
        assert_eq!(params[1].position, 1);
    }
}
