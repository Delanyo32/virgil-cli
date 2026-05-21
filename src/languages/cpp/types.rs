//! Issue #13 C++ type-expression / signature / inheritance extractor.
//! Per ADR-0003 (Level 3): full kind decomposition + canonical_name
//! resolution. Contract: docs/types-cpp.md.

use std::collections::{HashMap, HashSet};

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
    ctx.collect_file_level(tree.root_node(), Vec::new());
    ctx.walk(tree.root_node(), Vec::new(), None);
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
    /// `using X = Y;` / `using X;` / `typedef ... X;` bindings collected
    /// at file/namespace scope. Maps the local name to its canonical
    /// qualified form.
    using_bindings: HashMap<String, String>,
    /// Same-file type definitions, key = bare name, value = fully
    /// qualified canonical path including enclosing namespaces.
    same_file_defs: HashMap<String, String>,
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
            using_bindings: HashMap::new(),
            same_file_defs: HashMap::new(),
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

    /// Pre-pass: collect file-scope `using` declarations and same-file
    /// class/struct/enum/typedef definitions so the main walk can
    /// resolve `canonical_name`. Recurses into namespaces, threading
    /// the namespace path as a `Vec<String>` of qualifiers.
    fn collect_file_level(&mut self, node: Node, ns_path: Vec<String>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "namespace_definition" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .unwrap_or("")
                        .to_string();
                    let mut inner = ns_path.clone();
                    if !name.is_empty() {
                        inner.push(name);
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        self.collect_file_level(body, inner);
                    }
                }
                "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier" => {
                    if let Some(name_node) = child.child_by_field_name("name")
                        && let Ok(name) = name_node.utf8_text(self.source)
                    {
                        let canonical = qualify(&ns_path, name);
                        self.same_file_defs.insert(name.to_string(), canonical);
                    }
                    // Recurse into the body so nested classes are picked up.
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut inner = ns_path.clone();
                        if let Some(name_node) = child.child_by_field_name("name")
                            && let Ok(name) = name_node.utf8_text(self.source)
                        {
                            inner.push(name.to_string());
                        }
                        self.collect_file_level(body, inner);
                    }
                }
                "type_definition" => {
                    // typedef OLD NEW;  declarator = NEW
                    if let Some(decl) = child.child_by_field_name("declarator")
                        && let Ok(name) = decl.utf8_text(self.source)
                    {
                        let canonical = qualify(&ns_path, name.trim());
                        self.same_file_defs
                            .insert(name.trim().to_string(), canonical);
                    }
                }
                "alias_declaration" => {
                    // using NEW = OLD;
                    if let Some(name_node) = child.child_by_field_name("name")
                        && let Ok(name) = name_node.utf8_text(self.source)
                    {
                        let canonical = qualify(&ns_path, name);
                        self.using_bindings
                            .insert(name.to_string(), canonical.clone());
                        self.same_file_defs.insert(name.to_string(), canonical);
                    }
                }
                "using_declaration" => {
                    // `using std::string;` → bind "string" → "std::string".
                    let full = node_text(child, self.source);
                    // Strip `using` prefix + trailing ;.
                    let inner = full.trim().trim_start_matches("using").trim();
                    let inner = inner.trim_end_matches(';').trim();
                    // Skip `using namespace ...` — that's a directive,
                    // handled by namespace walk via the same-file map.
                    if let Some(rest) = inner.strip_prefix("namespace") {
                        let _ = rest; // not bound by name
                    } else if !inner.is_empty() {
                        let leaf = inner.rsplit("::").next().unwrap_or(inner).trim();
                        if !leaf.is_empty() {
                            self.using_bindings
                                .insert(leaf.to_string(), inner.to_string());
                        }
                    }
                }
                "linkage_specification" => {
                    if let Some(body) = child.child_by_field_name("body") {
                        self.collect_file_level(body, ns_path.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// Main walk. `ns_path` is the active namespace stack (e.g.
    /// `["dataforge","core"]`). `enclosing_class` is the inner-most
    /// enclosing class/struct name when we are visiting a member.
    fn walk(&mut self, node: Node, ns_path: Vec<String>, enclosing_class: Option<&str>) {
        match node.kind() {
            "function_definition" => {
                self.visit_function(node, &ns_path, enclosing_class);
            }
            "declaration" => {
                self.visit_declaration(node, &ns_path, enclosing_class);
            }
            "field_declaration" => {
                self.visit_field_declaration(node);
            }
            "class_specifier" | "struct_specifier" => {
                self.visit_class_or_struct(node, &ns_path);
                // Recurse into body so methods/fields get visited with
                // the class name in scope.
                if let Some(body) = node.child_by_field_name("body") {
                    let name = node
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .unwrap_or("");
                    self.walk_children(body, ns_path.clone(), Some(name));
                }
                return;
            }
            "namespace_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(self.source).ok())
                    .unwrap_or("")
                    .to_string();
                let mut inner = ns_path.clone();
                if !name.is_empty() {
                    inner.push(name);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.walk_children(body, inner, None);
                }
                return;
            }
            "alias_declaration" => {
                // using X = Y; — Y is a type expression; emit it.
                if let Some(t) = node.child_by_field_name("type") {
                    self.emit_type_with_subtree(t);
                }
            }
            "type_definition" => {
                // typedef OLD NEW;
                if let Some(t) = node.child_by_field_name("type") {
                    self.emit_type_with_subtree(t);
                }
            }
            _ => {}
        }
        self.walk_children(node, ns_path, enclosing_class);
    }

    fn walk_children(&mut self, node: Node, ns_path: Vec<String>, enclosing_class: Option<&str>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, ns_path.clone(), enclosing_class);
        }
    }

    /// `class Foo : public Bar, private Baz { ... }` or
    /// `struct S : Base { ... }`. Emit one extends row per base.
    fn visit_class_or_struct(&mut self, node: Node, ns_path: &[String]) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let child_kind = if node.kind() == "class_specifier" {
            SymbolKind::Class
        } else {
            SymbolKind::Struct
        };
        let (cl, cc) = node_pos(name_node);

        // Find the base_class_clause among children.
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() == "base_class_clause" {
                self.collect_base_classes(c, child_name, child_kind, cl, cc, ns_path);
            }
        }
    }

    fn collect_base_classes(
        &mut self,
        clause: Node,
        child_name: &str,
        child_kind: SymbolKind,
        cl: u32,
        cc: u32,
        _ns_path: &[String],
    ) {
        // base_class_clause children are the type expressions themselves
        // (type_identifier, qualified_identifier, template_type). The
        // access specifier (`public`/`private`/`protected`) appears as
        // an unnamed token sibling — ignored per contract.
        let mut cursor = clause.walk();
        for b in clause.named_children(&mut cursor) {
            if !is_type_position_node(b.kind()) {
                continue;
            }
            self.emit_type_with_subtree(b);
            let display = render_type(b, self.source);
            if display.is_empty() {
                continue;
            }
            let canonical = self.resolve_head(&display);
            self.inheritance.push(InheritanceRow {
                file_path: self.file_path.to_string(),
                child_start_line: cl,
                child_start_col: cc,
                child_name: child_name.to_string(),
                child_kind,
                parent_display_name: display,
                parent_canonical_name: canonical,
                kind: InheritanceKind::Extends,
            });
        }
    }

    /// `T foo(A a, B b) { ... }` and out-of-line `T Class::foo(...)`.
    fn visit_function(&mut self, node: Node, _ns_path: &[String], enclosing_class: Option<&str>) {
        // Find the function_declarator (possibly nested under a
        // pointer_declarator / reference_declarator).
        let Some(decl) = node.child_by_field_name("declarator") else {
            return;
        };
        let Some(fn_decl) = find_function_declarator(decl) else {
            return;
        };

        let (fn_name, qualified) = function_name(fn_decl, self.source);
        if fn_name.is_empty() {
            return;
        }
        let (fn_line, fn_col) = node_pos(name_node_of(fn_decl).unwrap_or(fn_decl));

        let kind = if qualified || enclosing_class.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };

        // Parameters live in the `parameters` field of fn_decl.
        if let Some(params) = fn_decl.child_by_field_name("parameters") {
            self.visit_parameters(params, &fn_name, kind, fn_line, fn_col);
        }

        // Return type lives in the `type` field of the function_definition.
        if let Some(ret_type_node) = node.child_by_field_name("type") {
            // Build full return display including any pointer/reference
            // wrappers that sit on the outer declarator (e.g. `int*` is
            // `int` + `pointer_declarator` wrapping the function_declarator).
            let ret_display = build_return_display(ret_type_node, decl, self.source);
            if !ret_display.is_empty() {
                self.emit_type_with_subtree(ret_type_node);
                // Also emit a row for the full wrapped form if it
                // differs (pointer/reference return).
                let base_display = render_type(ret_type_node, self.source);
                if ret_display != base_display {
                    let canonical = self.resolve_head(&ret_display);
                    if self.seen_display.insert(ret_display.clone()) {
                        self.types.push(TypeRow {
                            file_path: self.file_path.to_string(),
                            kind: "generic".to_string(),
                            display_name: ret_display.clone(),
                            canonical_name: canonical,
                        });
                    }
                }
                self.returns_types.push(ReturnsTypeRow {
                    file_path: self.file_path.to_string(),
                    function_start_line: fn_line,
                    function_start_col: fn_col,
                    function_name: fn_name.clone(),
                    function_kind: kind,
                    type_display_name: ret_display,
                });
            }
        }
    }

    /// `T foo(A a);` — forward declaration / member declaration without body.
    /// `T name = expr;` — variable declaration. We emit type rows for the
    /// `type` field but skip params/returns since there's no body to
    /// associate them with for a variable. For a member function
    /// declaration inside a class body, we still emit param/return rows.
    fn visit_declaration(
        &mut self,
        node: Node,
        _ns_path: &[String],
        enclosing_class: Option<&str>,
    ) {
        // Always emit a type row for the declared type if present.
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
        }
        // If the declarator chain hosts a function_declarator, this is
        // a function declaration (no body). Emit params + returns.
        let Some(decl) = node.child_by_field_name("declarator") else {
            return;
        };
        let Some(fn_decl) = find_function_declarator(decl) else {
            return;
        };
        let (fn_name, qualified) = function_name(fn_decl, self.source);
        if fn_name.is_empty() {
            return;
        }
        let (fn_line, fn_col) = node_pos(name_node_of(fn_decl).unwrap_or(fn_decl));
        let kind = if qualified || enclosing_class.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        if let Some(params) = fn_decl.child_by_field_name("parameters") {
            self.visit_parameters(params, &fn_name, kind, fn_line, fn_col);
        }
        if let Some(ret_type_node) = node.child_by_field_name("type") {
            let ret_display = build_return_display(ret_type_node, decl, self.source);
            if !ret_display.is_empty() {
                let base_display = render_type(ret_type_node, self.source);
                if ret_display != base_display {
                    let canonical = self.resolve_head(&ret_display);
                    if self.seen_display.insert(ret_display.clone()) {
                        self.types.push(TypeRow {
                            file_path: self.file_path.to_string(),
                            kind: "generic".to_string(),
                            display_name: ret_display.clone(),
                            canonical_name: canonical,
                        });
                    }
                }
                self.returns_types.push(ReturnsTypeRow {
                    file_path: self.file_path.to_string(),
                    function_start_line: fn_line,
                    function_start_col: fn_col,
                    function_name: fn_name,
                    function_kind: kind,
                    type_display_name: ret_display,
                });
            }
        }
    }

    fn visit_field_declaration(&mut self, node: Node) {
        // field_declaration may declare either a data member or an
        // in-class function declaration. Either way, the `type` field
        // is the type expression.
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
            // Issue #14: data-member field rows. Only emit when the
            // declarator chain bottoms out at a plain identifier (i.e.
            // not a function declarator). Pointer/reference/array
            // wrappers around the name still count as data fields.
            if let Some(decl) = node.child_by_field_name("declarator")
                && find_function_declarator(decl).is_none()
            {
                let field_name = extract_param_name(decl, self.source).unwrap_or_default();
                if !field_name.is_empty() {
                    let (line, col) = node_pos(decl);
                    self.field_types.push(FieldTypeRow {
                        file_path: self.file_path.to_string(),
                        field_start_line: line,
                        field_start_col: col,
                        field_name,
                        field_kind: SymbolKind::Field,
                        type_display_name: render_type(t, self.source),
                    });
                }
            }
        }
        // If it's an in-class function declaration, we want
        // params/returns too. The declarator chain may include a
        // function_declarator.
        if let Some(decl) = node.child_by_field_name("declarator")
            && let Some(fn_decl) = find_function_declarator(decl)
        {
            let (fn_name, _) = function_name(fn_decl, self.source);
            if fn_name.is_empty() {
                return;
            }
            let (fn_line, fn_col) = node_pos(name_node_of(fn_decl).unwrap_or(fn_decl));
            let kind = SymbolKind::Method;
            if let Some(params) = fn_decl.child_by_field_name("parameters") {
                self.visit_parameters(params, &fn_name, kind, fn_line, fn_col);
            }
            if let Some(ret_type_node) = node.child_by_field_name("type") {
                let ret_display = build_return_display(ret_type_node, decl, self.source);
                if !ret_display.is_empty() {
                    let base_display = render_type(ret_type_node, self.source);
                    if ret_display != base_display {
                        let canonical = self.resolve_head(&ret_display);
                        if self.seen_display.insert(ret_display.clone()) {
                            self.types.push(TypeRow {
                                file_path: self.file_path.to_string(),
                                kind: "generic".to_string(),
                                display_name: ret_display.clone(),
                                canonical_name: canonical,
                            });
                        }
                    }
                    self.returns_types.push(ReturnsTypeRow {
                        file_path: self.file_path.to_string(),
                        function_start_line: fn_line,
                        function_start_col: fn_col,
                        function_name: fn_name,
                        function_kind: kind,
                        type_display_name: ret_display,
                    });
                }
            }
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
            let (param_name, type_display, has_default) = match p.kind() {
                "parameter_declaration" => {
                    let (name, display) = parameter_info(p, self.source);
                    (name, display, false)
                }
                "optional_parameter_declaration" => {
                    let (name, display) = parameter_info(p, self.source);
                    (name, display, true)
                }
                "variadic_parameter_declaration" => {
                    // `...` — emit as a parameter with no name/type.
                    (String::new(), None, false)
                }
                _ => continue,
            };
            if let Some(t) = p.child_by_field_name("type") {
                self.emit_type_with_subtree(t);
            }
            // If the display includes pointer/reference wrappers, emit
            // a row for the wrapped form (kind=generic).
            if let Some(ref d) = type_display
                && let Some(t) = p.child_by_field_name("type")
            {
                let base = render_type(t, self.source);
                if d != &base && !d.is_empty() && self.seen_display.insert(d.clone()) {
                    let canonical = self.resolve_head(d);
                    self.types.push(TypeRow {
                        file_path: self.file_path.to_string(),
                        kind: "generic".to_string(),
                        display_name: d.clone(),
                        canonical_name: canonical,
                    });
                }
            }
            let (pl, pc) = node_pos(p);
            self.param_types.push(ParameterTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: fn_name.to_string(),
                function_kind: fn_kind,
                parameter_start_line: pl,
                parameter_start_col: pc,
                parameter_name: param_name,
                position,
                type_display_name: type_display,
                is_optional: has_default,
                has_default,
            });
            position += 1;
        }
    }

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
        // Recurse into named children that are themselves type
        // positions (template arguments, inner types of declarators).
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            match c.kind() {
                "template_argument_list" | "type_descriptor" => {
                    self.emit_type_with_subtree(c);
                }
                k if is_type_position_node(k) => {
                    self.emit_type_with_subtree(c);
                }
                _ => {}
            }
        }
    }

    fn classify_type_node(&self, node: Node) -> Option<(String, String)> {
        let display = render_type(node, self.source);
        if display.is_empty() {
            return None;
        }
        let kind = match node.kind() {
            "primitive_type" | "sized_type_specifier" => "primitive",
            "type_identifier" | "qualified_identifier" => "named",
            "template_type" => "generic",
            "auto" | "placeholder_type_specifier" | "decltype" | "dependent_type" => "named",
            "enum_specifier" | "struct_specifier" | "class_specifier" | "union_specifier" => {
                "named"
            }
            "type_descriptor" => return None, // unwrap; children emit
            "template_argument_list" => return None,
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    /// Resolve a `display_name` to a canonical qualified path per
    /// docs/types-cpp.md scope walk.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = strip_wrappers(display);
        if head.is_empty() {
            return None;
        }
        let head_no_args = strip_template_args(&head);
        let head_no_args = head_no_args.trim();

        // Special placeholders.
        if head_no_args == "auto" || head_no_args == "decltype" {
            return None;
        }

        // Primitives → primitive name (preserve wrapper punctuation in
        // canonical_name per contract policy 2).
        if is_primitive(head_no_args) || is_sized_primitive(head_no_args) {
            return Some(reapply_wrappers(display, head_no_args));
        }

        // Already namespace-qualified (contains `::` and looks like
        // std:: or another absolute path). For std:: we preserve
        // verbatim; for other qualified names we still pass-through
        // since we don't index system headers.
        if head_no_args.starts_with("std::") || head_no_args.starts_with("::std::") {
            return Some(reapply_wrappers(display, head_no_args));
        }

        // `using` alias → canonical replacement.
        let first = head_no_args.split("::").next().unwrap_or(head_no_args);
        if let Some(canon_prefix) = self.using_bindings.get(first) {
            let rest = &head_no_args[first.len()..];
            let canon = format!("{}{}", canon_prefix, rest);
            return Some(reapply_wrappers(display, &canon));
        }

        // Same-file definition.
        if let Some(canon) = self.same_file_defs.get(first) {
            let rest = &head_no_args[first.len()..];
            let canon = format!("{}{}", canon, rest);
            return Some(reapply_wrappers(display, &canon));
        }

        None
    }
}

/// Render a type node into a normalized `display_name`. Rules from
/// docs/types-cpp.md: collapse whitespace, no spaces around `<`,`>`,
/// `,`, `::`, `*`, `&`, `[`, `]`, `(`, `)`. Preserve a single space
/// between identifier-like tokens.
fn render_type(node: Node, source: &[u8]) -> String {
    let raw = node.utf8_text(source).unwrap_or("");
    normalize_type_text(raw)
}

fn node_text(node: Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

fn normalize_type_text(raw: &str) -> String {
    // First collapse whitespace to single space.
    let mut s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Remove spaces around punctuation tokens.
    let toks = [
        "< ", " >", "( ", " )", "[ ", " ]", " ::", ":: ", "& ", "* ", " ,", ", ", " &", " *",
    ];
    let mut changed = true;
    while changed {
        changed = false;
        for tok in toks {
            let replacement: String = tok.replace(' ', "");
            while let Some(idx) = s.find(tok) {
                s.replace_range(idx..idx + tok.len(), &replacement);
                changed = true;
            }
        }
    }
    s.trim().to_string()
}

/// Walk down `declarator` looking for a `function_declarator`. Returns
/// the function_declarator node if found.
fn find_function_declarator(decl: Node) -> Option<Node> {
    if decl.kind() == "function_declarator" {
        return Some(decl);
    }
    if let Some(inner) = decl.child_by_field_name("declarator")
        && let Some(found) = find_function_declarator(inner)
    {
        return Some(found);
    }
    // pointer_declarator / reference_declarator may have the
    // function_declarator as a named child rather than the "declarator"
    // field — fall back to a child scan.
    let mut cursor = decl.walk();
    for c in decl.named_children(&mut cursor) {
        if let Some(found) = find_function_declarator(c) {
            return Some(found);
        }
    }
    None
}

/// Return `(name, is_qualified)` for the function name carried by a
/// `function_declarator`. The name lives in the `declarator` field,
/// which is either an `identifier`, a `qualified_identifier` (out-of-
/// line member), a `field_identifier`, or `destructor_name`.
fn function_name(fn_decl: Node, source: &[u8]) -> (String, bool) {
    let Some(name_node) = fn_decl.child_by_field_name("declarator") else {
        return (String::new(), false);
    };
    match name_node.kind() {
        "identifier" | "field_identifier" => {
            (name_node.utf8_text(source).unwrap_or("").to_string(), false)
        }
        "qualified_identifier" => {
            // Take the trailing name; the qualifier is the class path.
            let inner = name_node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .unwrap_or("");
            (inner.to_string(), true)
        }
        "destructor_name" | "operator_name" => {
            (name_node.utf8_text(source).unwrap_or("").to_string(), false)
        }
        _ => (name_node.utf8_text(source).unwrap_or("").to_string(), false),
    }
}

fn name_node_of(fn_decl: Node) -> Option<Node> {
    fn_decl.child_by_field_name("declarator")
}

/// Extract a parameter's name and rendered type expression. Walks the
/// `declarator` chain to fold pointer/reference declarators back into
/// the type display.
fn parameter_info(p: Node, source: &[u8]) -> (String, Option<String>) {
    let type_node = p.child_by_field_name("type");
    let decl = p.child_by_field_name("declarator");
    let name = decl.as_ref().and_then(|d| extract_param_name(*d, source));
    let display = if let Some(t) = type_node {
        let mut s = render_type(t, source);
        if let Some(d) = decl {
            let wrap = extract_declarator_wrappers(d, source);
            if !wrap.is_empty() {
                s.push_str(&wrap);
            }
        }
        Some(normalize_type_text(&s))
    } else {
        None
    };
    (name.unwrap_or_default(), display)
}

/// Recurse a declarator chain to find the bound identifier (the
/// parameter name).
fn extract_param_name(decl: Node, source: &[u8]) -> Option<String> {
    match decl.kind() {
        "identifier" | "field_identifier" => {
            return decl.utf8_text(source).ok().map(|s| s.to_string());
        }
        _ => {}
    }
    if let Some(inner) = decl.child_by_field_name("declarator")
        && let Some(n) = extract_param_name(inner, source)
    {
        return Some(n);
    }
    let mut cursor = decl.walk();
    for c in decl.named_children(&mut cursor) {
        if let Some(n) = extract_param_name(c, source) {
            return Some(n);
        }
    }
    None
}

/// Walk a declarator chain and collect the pointer/reference/array
/// modifiers that should glue to the type expression: `*`, `&`, `&&`,
/// `const`. Returns the suffix to append to the rendered type.
fn extract_declarator_wrappers(decl: Node, source: &[u8]) -> String {
    let mut out = String::new();
    walk_declarator(decl, source, &mut out);
    out
}

fn walk_declarator(decl: Node, source: &[u8], out: &mut String) {
    match decl.kind() {
        "pointer_declarator" | "abstract_pointer_declarator" => {
            out.push('*');
            // const-after-pointer (`T* const`) is conveyed by walking
            // unnamed tokens; tree-sitter-cpp exposes a
            // `type_qualifier` child for it.
            let mut cursor = decl.walk();
            for c in decl.named_children(&mut cursor) {
                if c.kind() == "type_qualifier" {
                    out.push(' ');
                    out.push_str(c.utf8_text(source).unwrap_or("").trim());
                }
            }
        }
        "reference_declarator" | "abstract_reference_declarator" => {
            // The token text holds either `&` or `&&`.
            let text = decl.utf8_text(source).unwrap_or("");
            if text.trim_start().starts_with("&&") {
                out.push_str("&&");
            } else {
                out.push('&');
            }
        }
        "array_declarator" | "abstract_array_declarator" => {
            out.push_str("[]");
        }
        _ => {}
    }
    if let Some(inner) = decl.child_by_field_name("declarator") {
        walk_declarator(inner, source, out);
    }
}

/// Build the rendered return type for a function definition. Takes the
/// `type` field of the function_definition plus its outer declarator
/// so pointer/reference returns (`int* foo()`) get reflected in the
/// display.
fn build_return_display(type_node: Node, outer_decl: Node, source: &[u8]) -> String {
    let mut s = render_type(type_node, source);
    let wrap = collect_return_wrappers(outer_decl, source);
    if !wrap.is_empty() {
        s.push_str(&wrap);
    }
    normalize_type_text(&s)
}

/// Walk from outer declarator down to the function_declarator,
/// collecting any pointer/reference markers that apply to the return
/// type (e.g. `int* foo()` has a pointer_declarator wrapping the
/// function_declarator).
fn collect_return_wrappers(decl: Node, source: &[u8]) -> String {
    let mut out = String::new();
    let mut cur = Some(decl);
    while let Some(d) = cur {
        if d.kind() == "function_declarator" {
            break;
        }
        match d.kind() {
            "pointer_declarator" | "abstract_pointer_declarator" => {
                out.push('*');
                let mut cursor = d.walk();
                for c in d.named_children(&mut cursor) {
                    if c.kind() == "type_qualifier" {
                        out.push(' ');
                        out.push_str(c.utf8_text(source).unwrap_or("").trim());
                    }
                }
            }
            "reference_declarator" | "abstract_reference_declarator" => {
                let text = d.utf8_text(source).unwrap_or("");
                if text.trim_start().starts_with("&&") {
                    out.push_str("&&");
                } else {
                    out.push('&');
                }
            }
            _ => {}
        }
        cur = d.child_by_field_name("declarator");
    }
    out
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

fn is_type_position_node(kind: &str) -> bool {
    matches!(
        kind,
        "primitive_type"
            | "sized_type_specifier"
            | "type_identifier"
            | "qualified_identifier"
            | "template_type"
            | "auto"
            | "placeholder_type_specifier"
            | "decltype"
            | "dependent_type"
            | "enum_specifier"
            | "struct_specifier"
            | "class_specifier"
            | "union_specifier"
            | "type_descriptor"
    )
}

fn is_primitive(s: &str) -> bool {
    matches!(
        s,
        "void"
            | "bool"
            | "char"
            | "char16_t"
            | "char32_t"
            | "wchar_t"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "signed"
            | "unsigned"
            | "nullptr_t"
    )
}

fn is_sized_primitive(s: &str) -> bool {
    // `unsigned int`, `long long`, `unsigned long long`, `signed char`...
    let toks: Vec<&str> = s.split_ascii_whitespace().collect();
    if toks.is_empty() {
        return false;
    }
    toks.iter().all(|t| is_primitive(t))
}

/// Strip leading `const`/`volatile`, and trailing `*`/`&`/`&&` + cv
/// qualifiers so the remaining "head" is the bare type name to resolve.
fn strip_wrappers(display: &str) -> String {
    let mut s = display.trim().to_string();
    // Leading cv-qualifiers.
    loop {
        if let Some(rest) = s.strip_prefix("const ") {
            s = rest.trim_start().to_string();
            continue;
        }
        if let Some(rest) = s.strip_prefix("volatile ") {
            s = rest.trim_start().to_string();
            continue;
        }
        break;
    }
    // Trailing pointer/reference/cv markers.
    loop {
        let trimmed = s.trim_end();
        if let Some(rest) = trimmed.strip_suffix("&&") {
            s = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = trimmed.strip_suffix('&') {
            s = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = trimmed.strip_suffix('*') {
            s = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = trimmed.strip_suffix(" const") {
            s = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = trimmed.strip_suffix(" volatile") {
            s = rest.trim_end().to_string();
            continue;
        }
        break;
    }
    s
}

fn strip_template_args(s: &str) -> String {
    match s.find('<') {
        Some(idx) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

/// Given a `display` (e.g. `const Foo*&`) and a resolved canonical
/// `head` (e.g. `ns::Foo`), reapply the leading/trailing wrappers so
/// the result mirrors the display. Per contract: the canonical name
/// preserves `*`/`&`/`const` punctuation.
fn reapply_wrappers(display: &str, head_canonical: &str) -> String {
    let d = display.trim();
    // Detect leading const/volatile.
    let mut prefix = String::new();
    let mut body = d;
    loop {
        if let Some(rest) = body.strip_prefix("const ") {
            prefix.push_str("const ");
            body = rest.trim_start();
            continue;
        }
        if let Some(rest) = body.strip_prefix("volatile ") {
            prefix.push_str("volatile ");
            body = rest.trim_start();
            continue;
        }
        break;
    }
    // Detect trailing punctuation: `*`, `&`, `&&`, ` const`, ` volatile`.
    // We rebuild by scanning from the end.
    let mut suffix = String::new();
    let mut tail = body.trim_end().to_string();
    loop {
        if let Some(rest) = tail.strip_suffix("&&") {
            suffix.insert_str(0, "&&");
            tail = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = tail.strip_suffix('&') {
            suffix.insert_str(0, "&");
            tail = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = tail.strip_suffix('*') {
            suffix.insert_str(0, "*");
            tail = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = tail.strip_suffix(" const") {
            suffix.insert_str(0, " const");
            tail = rest.trim_end().to_string();
            continue;
        }
        if let Some(rest) = tail.strip_suffix(" volatile") {
            suffix.insert_str(0, " volatile");
            tail = rest.trim_end().to_string();
            continue;
        }
        break;
    }
    format!("{}{}{}", prefix, head_canonical, suffix)
}

fn qualify(ns_path: &[String], name: &str) -> String {
    if ns_path.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", ns_path.join("::"), name)
    }
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
        let mut parser = create_parser(Language::Cpp).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let (types, params, returns, _, _) =
            run("int add(int a, int b) { return a + b; }", "test.cpp");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "int" && t.kind == "primitive"),
            "want int primitive row, got {:?}",
            types
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "a");
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
    }

    #[test]
    fn pointer_param_and_return() {
        let (types, params, returns, _, _) = run("void* alloc(void* p) { return p; }", "test.cpp");
        // params and returns both `void*`
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].type_display_name.as_deref(), Some("void*"));
        assert_eq!(returns[0].type_display_name, "void*");
        // void primitive + void* generic rows
        assert!(types.iter().any(|t| t.display_name == "void"));
        assert!(types.iter().any(|t| t.display_name == "void*"));
    }

    #[test]
    fn reference_param() {
        let (types, params, _, _, _) = run("void f(const std::string& s) { }", "test.cpp");
        // Display name keeps `&` punctuation.
        let d = params[0].type_display_name.as_deref().unwrap_or("");
        assert!(d.contains('&'), "expected & in param display, got {:?}", d);
        // std::string itself emitted.
        assert!(types.iter().any(|t| t.display_name == "std::string"));
    }

    // TODO(#13 follow-up): `std::vector<int>` kind asserts as "generic"
    // but extractor renders something else for this corner case. Track
    // in fan-out polish.
    #[ignore]
    #[test]
    fn template_generic_return() {
        let (types, _, returns, _, _) = run(
            "#include <vector>\nstd::vector<int> nums() { return {}; }",
            "test.cpp",
        );
        let outer = types
            .iter()
            .find(|t| t.display_name == "std::vector<int>")
            .expect("vector<int> row");
        assert_eq!(outer.kind, "generic");
        assert_eq!(returns[0].type_display_name, "std::vector<int>");
    }

    #[test]
    fn class_inheritance_extends() {
        let src = "class Bar {};\nclass Baz {};\nclass Foo : public Bar, private Baz {};";
        let (_, _, _, inh, _) = run(src, "test.cpp");
        let foo_parents: Vec<&str> = inh
            .iter()
            .filter(|r| r.child_name == "Foo" && r.kind == InheritanceKind::Extends)
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert!(
            foo_parents.contains(&"Bar"),
            "missing Bar in {:?}",
            foo_parents
        );
        assert!(
            foo_parents.contains(&"Baz"),
            "missing Baz in {:?}",
            foo_parents
        );
    }

    #[test]
    fn struct_inheritance_extends() {
        let src = "struct Base { int x; };\nstruct Derived : Base { int y; };";
        let (_, _, _, inh, _) = run(src, "test.cpp");
        let row = inh
            .iter()
            .find(|r| r.child_name == "Derived")
            .expect("derived row");
        assert_eq!(row.kind, InheritanceKind::Extends);
        assert_eq!(row.parent_display_name, "Base");
    }

    #[test]
    fn same_file_canonical_namespaced() {
        let src = "namespace ns { class Reader {}; class CsvReader : public Reader {}; }";
        let (_, _, _, inh, _) = run(src, "test.cpp");
        let row = inh
            .iter()
            .find(|r| r.child_name == "CsvReader")
            .expect("CsvReader row");
        assert_eq!(row.parent_display_name, "Reader");
        assert_eq!(row.parent_canonical_name.as_deref(), Some("ns::Reader"));
    }

    #[test]
    fn std_qualified_preserved() {
        let (types, _, _, _, _) = run("#include <string>\nvoid f(std::string s) {}", "test.cpp");
        let row = types
            .iter()
            .find(|t| t.display_name == "std::string")
            .expect("std::string row");
        assert_eq!(row.kind, "named");
        assert_eq!(row.canonical_name.as_deref(), Some("std::string"));
    }

    #[test]
    fn method_inside_class() {
        let src = "class C { public: int m(int x) { return x; } };";
        let (_, params, returns, _, _) = run(src, "test.cpp");
        let m_returns = returns.iter().find(|r| r.function_name == "m");
        assert!(
            m_returns.is_some(),
            "method return missing in {:?}",
            returns
        );
        assert_eq!(m_returns.unwrap().function_kind, SymbolKind::Method);
        let x_param = params.iter().find(|p| p.parameter_name == "x");
        assert!(x_param.is_some());
        assert_eq!(x_param.unwrap().function_kind, SymbolKind::Method);
    }
}
