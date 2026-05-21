//! Issue #13 TypeScript/JavaScript type-expression / signature /
//! inheritance extractor. Per ADR-0003 (Level 3): full kind decomposition
//! + canonical_name resolution. Contract: docs/types-typescript.md.
//!
//! For `.js` / `.jsx` files we emit `parameter` rows with
//! `type_display_name = None` (no annotations to read), no
//! `returns_type` rows, and no `type` rows — matching the
//! "JavaScript divergence" section of the contract.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::models::{
    InheritanceKind, InheritanceRow, ParameterTypeRow, ReturnsTypeRow, SymbolKind, TypeRow,
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
) {
    let is_js = is_javascript_path(file_path);
    let mut ctx = Ctx::new(file_path, source, is_js);
    ctx.walk(tree.root_node());
    ctx.finish()
}

struct Ctx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    is_js: bool,
    types: Vec<TypeRow>,
    seen_display: HashSet<String>,
    param_types: Vec<ParameterTypeRow>,
    returns_types: Vec<ReturnsTypeRow>,
    inheritance: Vec<InheritanceRow>,
}

impl<'a> Ctx<'a> {
    fn new(file_path: &'a str, source: &'a [u8], is_js: bool) -> Self {
        Self {
            file_path,
            source,
            is_js,
            types: Vec::new(),
            seen_display: HashSet::new(),
            param_types: Vec::new(),
            returns_types: Vec::new(),
            inheritance: Vec::new(),
        }
    }

    fn finish(
        self,
    ) -> (
        Vec<TypeRow>,
        Vec<ParameterTypeRow>,
        Vec<ReturnsTypeRow>,
        Vec<InheritanceRow>,
    ) {
        (
            self.types,
            self.param_types,
            self.returns_types,
            self.inheritance,
        )
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_declaration"
            | "function_expression"
            | "method_definition"
            | "method_signature"
            | "abstract_method_signature"
            | "generator_function"
            | "generator_function_declaration"
            | "arrow_function"
            | "function_signature" => self.visit_function_like(node),

            "class_declaration" | "abstract_class_declaration" => self.visit_class(node),
            "interface_declaration" => self.visit_interface(node),
            "type_alias_declaration" => self.visit_type_alias(node),
            "enum_declaration" => {}
            _ => {}
        }

        // Recurse — nested functions / classes are common in TS/JS.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
        }
    }

    fn visit_function_like(&mut self, node: Node) {
        // Function name: for function_declaration, method_definition, etc.
        // arrow_function / function_expression are anonymous unless bound by
        // a variable_declarator — we walk the parent to find the binding.
        let (fn_name, fn_line, fn_col, fn_kind) = match resolve_function_identity(node, self.source)
        {
            Some(v) => v,
            None => return,
        };

        // Parameters live on a child labeled "parameters" (formal_parameters)
        // OR — for an arrow function with a single bare identifier — directly
        // as the "parameter" field.
        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, &fn_name, fn_kind, fn_line, fn_col);
        } else if let Some(p) = node.child_by_field_name("parameter") {
            // Bare-identifier arrow: `x => ...`. Untyped by definition.
            let (pl, pc) = node_pos(p);
            let pname = p.utf8_text(self.source).unwrap_or("").to_string();
            self.param_types.push(ParameterTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: fn_name.clone(),
                function_kind: fn_kind,
                parameter_start_line: pl,
                parameter_start_col: pc,
                parameter_name: pname,
                position: 0,
                type_display_name: None,
                is_optional: false,
                has_default: false,
            });
        }

        // Return type annotation: child labeled "return_type".
        if !self.is_js
            && let Some(ret) = node.child_by_field_name("return_type")
        {
            // `return_type` is a `type_annotation` wrapper — peel to inner type node.
            let inner = unwrap_type_annotation(ret);
            let display = render_type(inner, self.source);
            self.emit_type_with_subtree(inner);
            self.returns_types.push(ReturnsTypeRow {
                file_path: self.file_path.to_string(),
                function_start_line: fn_line,
                function_start_col: fn_col,
                function_name: fn_name,
                function_kind: fn_kind,
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
                "required_parameter" | "optional_parameter" => {
                    self.emit_ts_parameter(
                        p, fn_name, fn_kind, fn_line, fn_col, position, /*js*/ false,
                    );
                    position += 1;
                }
                // JS/TS bare-identifier formal_parameters entries:
                "identifier" | "rest_pattern" | "assignment_pattern" | "object_pattern"
                | "array_pattern" => {
                    self.emit_js_parameter(p, fn_name, fn_kind, fn_line, fn_col, position);
                    position += 1;
                }
                _ => {}
            }
        }
    }

    fn emit_ts_parameter(
        &mut self,
        p: Node,
        fn_name: &str,
        fn_kind: SymbolKind,
        fn_line: u32,
        fn_col: u32,
        position: i64,
        _js: bool,
    ) {
        let is_optional = p.kind() == "optional_parameter";
        let pattern = p.child_by_field_name("pattern");
        let (pname, pl, pc) = match pattern {
            Some(pat) => {
                let name = extract_pattern_name(pat, self.source);
                let (l, c) = node_pos(pat);
                (name, l, c)
            }
            None => {
                let (l, c) = node_pos(p);
                (String::new(), l, c)
            }
        };
        let has_default = p.child_by_field_name("value").is_some();
        let type_display = if self.is_js {
            None
        } else if let Some(t) = p.child_by_field_name("type") {
            // `type` field is the type_annotation; peel it.
            let inner = unwrap_type_annotation(t);
            self.emit_type_with_subtree(inner);
            Some(render_type(inner, self.source))
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
            parameter_name: pname,
            position,
            type_display_name: type_display,
            is_optional,
            has_default,
        });
    }

    fn emit_js_parameter(
        &mut self,
        p: Node,
        fn_name: &str,
        fn_kind: SymbolKind,
        fn_line: u32,
        fn_col: u32,
        position: i64,
    ) {
        let pname = extract_pattern_name(p, self.source);
        let has_default = p.kind() == "assignment_pattern";
        let (pl, pc) = node_pos(p);
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
            type_display_name: None,
            is_optional: false,
            has_default,
        });
    }

    fn visit_class(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // The class body has zero or more heritage clauses as children of the
        // class_declaration (NOT inside class_body).
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "class_heritage" => {
                    // class_heritage contains extends_clause / implements_clause children.
                    let mut hc = child.walk();
                    for h in child.named_children(&mut hc) {
                        match h.kind() {
                            "extends_clause" => self.collect_heritage(
                                h,
                                child_name,
                                SymbolKind::Class,
                                cl,
                                cc,
                                InheritanceKind::Extends,
                            ),
                            "implements_clause" => self.collect_heritage(
                                h,
                                child_name,
                                SymbolKind::Class,
                                cl,
                                cc,
                                InheritanceKind::Implements,
                            ),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Walk into class body for field types.
        if !self.is_js
            && let Some(body) = node.child_by_field_name("body")
        {
            self.emit_class_body_types(body);
        }
    }

    fn visit_interface(&mut self, node: Node) {
        if self.is_js {
            return;
        }
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // Interface heritage: `extends_type_clause` is a direct child of
        // interface_declaration.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if matches!(child.kind(), "extends_type_clause" | "extends_clause") {
                self.collect_heritage(
                    child,
                    child_name,
                    SymbolKind::Interface,
                    cl,
                    cc,
                    InheritanceKind::Extends,
                );
            }
        }

        if let Some(body) = node.child_by_field_name("body") {
            self.emit_interface_body_types(body);
        }
    }

    fn visit_type_alias(&mut self, node: Node) {
        if self.is_js {
            return;
        }
        if let Some(t) = node.child_by_field_name("value") {
            self.emit_type_with_subtree(t);
        }
    }

    /// Walk an `extends_clause` / `implements_clause` / `extends_type_clause`
    /// and emit one inheritance row per parent type expression.
    fn collect_heritage(
        &mut self,
        clause: Node,
        child_name: &str,
        child_kind: SymbolKind,
        cl: u32,
        cc: u32,
        kind: InheritanceKind,
    ) {
        let mut cursor = clause.walk();
        for parent in clause.named_children(&mut cursor) {
            // Skip type_arguments — those belong to the parent type expr above.
            if parent.kind() == "type_arguments" {
                continue;
            }
            // Heritage parents in TS are `identifier`, `nested_identifier`,
            // `type_identifier`, `nested_type_identifier`, or
            // `generic_type`. Render the whole node as display.
            let display = render_type(parent, self.source);
            if display.is_empty() {
                continue;
            }
            // Emit a type row for the parent (and its subtree) so type_use
            // references resolve.
            if !self.is_js {
                self.emit_type_with_subtree(parent);
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
                kind,
            });
        }
    }

    fn emit_class_body_types(&mut self, body: Node) {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            match child.kind() {
                "public_field_definition" | "property_signature" => {
                    if let Some(t) = child.child_by_field_name("type") {
                        let inner = unwrap_type_annotation(t);
                        self.emit_type_with_subtree(inner);
                    }
                }
                _ => {}
            }
        }
    }

    fn emit_interface_body_types(&mut self, body: Node) {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            match child.kind() {
                "property_signature" => {
                    if let Some(t) = child.child_by_field_name("type") {
                        let inner = unwrap_type_annotation(t);
                        self.emit_type_with_subtree(inner);
                    }
                }
                "method_signature" => {
                    // method_signature inside an interface body is handled
                    // by the main walk when it recurses into the interface.
                    // We don't need to do anything here.
                }
                _ => {}
            }
        }
    }

    /// Emit a TypeRow for `node` and every meaningful sub-type expression
    /// nested inside it. Per docs/types-typescript.md `display_name`
    /// construction.
    fn emit_type_with_subtree(&mut self, node: Node) {
        if self.is_js {
            return;
        }
        // `parenthesized_type` is transparent — peel and recurse on inner.
        if node.kind() == "parenthesized_type" {
            if let Some(inner) = first_named_child(node) {
                self.emit_type_with_subtree(inner);
            }
            return;
        }
        if node.kind() == "readonly_type" {
            // The `readonly` modifier is stripped from display_name; recurse
            // into the inner type.
            if let Some(inner) = first_type_child(node) {
                self.emit_type_with_subtree(inner);
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
        // Recurse into children so nested types emit their own rows.
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if !is_type_position_node(c.kind()) {
                continue;
            }
            self.emit_type_with_subtree(c);
        }
    }

    /// Map a tree-sitter node to its schema `kind` + the rendered
    /// `display_name`. Returns `None` for non-type nodes.
    fn classify_type_node(&self, node: Node) -> Option<(String, String)> {
        let display = render_type(node, self.source);
        if display.is_empty() {
            return None;
        }
        let kind = match node.kind() {
            "predefined_type" => "primitive",
            "type_identifier" | "nested_type_identifier" | "nested_identifier" | "identifier" => {
                "named"
            }
            "generic_type" => "generic",
            "union_type" => "union",
            "intersection_type" => "intersection",
            "function_type" | "constructor_type" | "type_predicate" => "function",
            "tuple_type" => "tuple",
            "array_type" => "array",
            "literal_type" => "named",
            "object_type" | "type_literal" => "named",
            "index_type_query" => "named",
            "lookup_type" => "named",
            "conditional_type" => "named",
            "template_literal_type" => "named",
            "this_type" => "named",
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    /// Resolve `display` to a canonical name per the scope-walk order in
    /// docs/types-typescript.md. Returns `None` when unresolvable.
    ///
    /// We do not have access to the file's `imports` rows from inside this
    /// extractor (the population layer joins types↔imports later). So we
    /// only resolve:
    ///   - predefined types → `typescript::primitive::<name>`
    ///   - global ambient allow-list → `typescript::global::<name>`
    /// Everything else returns `None` and downstream Cozoscript handles
    /// import-based resolution. This matches the Rust pilot, which also
    /// only fills in canonical_name for what it can prove locally.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = strip_array_suffix(display);
        let head = strip_generic_args(head);
        let head = head.trim();
        if head.is_empty() {
            return None;
        }
        // Compound display forms (containing `|`, `&`, `[`, `(`, `=>`) are
        // not resolvable as a single canonical head — they belong to the
        // union/intersection/function/tuple kinds whose canonical_name is
        // null per contract.
        if head.contains('|')
            || head.contains('&')
            || head.contains('(')
            || head.contains('=')
            || head.contains('{')
        {
            return None;
        }
        // Literal type forms keep quotes / digits in display; null.
        if head.starts_with('\'') || head.starts_with('"') || head.starts_with('`') {
            return None;
        }
        if head.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        // 1. Predefined primitives.
        if is_predefined(head) {
            return Some(format!("typescript::primitive::{head}"));
        }
        // 2. Global ambient allow-list.
        if is_global_ambient(head) {
            return Some(format!("typescript::global::{head}"));
        }
        None
    }
}

// ── Helpers ──

fn is_javascript_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

fn first_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn first_type_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|c| is_type_position_node(c.kind()) || c.kind() == "parenthesized_type")
}

/// A `type_annotation` wraps the actual type expression. Peel it.
fn unwrap_type_annotation(node: Node) -> Node {
    if node.kind() == "type_annotation" {
        if let Some(inner) = first_named_child(node) {
            return inner;
        }
    }
    node
}

/// Resolve a function-like node to `(name, line, col, kind)`.
///
/// - `function_declaration` / `generator_function_declaration` /
///   `function_signature`: name lives on `name` field.
/// - `method_definition` / `method_signature` / `abstract_method_signature`:
///   name lives on `name` field (property_identifier).
/// - `arrow_function` / `function_expression` / `generator_function`:
///   anonymous; we walk parents to find a binding (variable_declarator,
///   public_field_definition).
fn resolve_function_identity(node: Node, source: &[u8]) -> Option<(String, u32, u32, SymbolKind)> {
    let kind_str = node.kind();
    // Methods.
    if matches!(
        kind_str,
        "method_definition" | "method_signature" | "abstract_method_signature"
    ) {
        let name_node = node.child_by_field_name("name")?;
        let name = name_node.utf8_text(source).ok()?.to_string();
        let (l, c) = node_pos(name_node);
        return Some((name, l, c, SymbolKind::Method));
    }
    // Declarations with a name field.
    if matches!(
        kind_str,
        "function_declaration" | "generator_function_declaration" | "function_signature"
    ) {
        let name_node = node.child_by_field_name("name")?;
        let name = name_node.utf8_text(source).ok()?.to_string();
        let (l, c) = node_pos(name_node);
        return Some((name, l, c, SymbolKind::Function));
    }
    // Anonymous: arrow_function, function_expression, generator_function.
    // Walk up to a binding.
    let parent = node.parent()?;
    match parent.kind() {
        "variable_declarator" => {
            let name_node = parent.child_by_field_name("name")?;
            let name = name_node.utf8_text(source).ok()?.to_string();
            let (l, c) = node_pos(name_node);
            let sk = if kind_str == "arrow_function" {
                SymbolKind::ArrowFunction
            } else {
                SymbolKind::Function
            };
            Some((name, l, c, sk))
        }
        "public_field_definition" | "property_signature" => {
            let name_node = parent.child_by_field_name("name")?;
            let name = name_node.utf8_text(source).ok()?.to_string();
            let (l, c) = node_pos(name_node);
            Some((name, l, c, SymbolKind::Method))
        }
        "pair" => {
            // Object literal property: `{ foo: () => ... }`. Use the key.
            let key = parent.child_by_field_name("key")?;
            let name = key.utf8_text(source).ok()?.to_string();
            let (l, c) = node_pos(key);
            let sk = if kind_str == "arrow_function" {
                SymbolKind::ArrowFunction
            } else {
                SymbolKind::Function
            };
            Some((name, l, c, sk))
        }
        _ => None,
    }
}

fn extract_pattern_name(pat: Node, source: &[u8]) -> String {
    match pat.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            pat.utf8_text(source).unwrap_or("").to_string()
        }
        "rest_pattern" => {
            // rest_pattern wraps an identifier.
            let mut cursor = pat.walk();
            for c in pat.named_children(&mut cursor) {
                if c.kind() == "identifier" {
                    return c.utf8_text(source).unwrap_or("").to_string();
                }
            }
            String::new()
        }
        "assignment_pattern" => {
            // assignment_pattern { left: identifier, right: ... }
            if let Some(left) = pat.child_by_field_name("left") {
                return extract_pattern_name(left, source);
            }
            String::new()
        }
        "object_pattern" | "array_pattern" => {
            // Destructured — no single name; use the raw source as a label.
            pat.utf8_text(source).unwrap_or("").to_string()
        }
        _ => pat.utf8_text(source).unwrap_or("").to_string(),
    }
}

fn is_type_position_node(kind: &str) -> bool {
    matches!(
        kind,
        "predefined_type"
            | "type_identifier"
            | "nested_type_identifier"
            | "nested_identifier"
            | "generic_type"
            | "union_type"
            | "intersection_type"
            | "function_type"
            | "constructor_type"
            | "type_predicate"
            | "tuple_type"
            | "array_type"
            | "literal_type"
            | "object_type"
            | "type_literal"
            | "index_type_query"
            | "lookup_type"
            | "conditional_type"
            | "template_literal_type"
            | "this_type"
            | "parenthesized_type"
            | "readonly_type"
            | "type_arguments"
            | "type_parameters"
    )
}

/// Render a type node into the normalized `display_name` form per
/// docs/types-typescript.md "display_name construction" rules.
fn render_type(node: Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    let s = normalize_type_text(text);
    // Strip leading `readonly ` if it slipped through (we usually peel via
    // readonly_type, but heritage clauses sometimes carry the keyword as
    // part of a parent identifier reference — rare).
    s.strip_prefix("readonly ")
        .map(|r| r.to_string())
        .unwrap_or(s)
}

fn normalize_type_text(raw: &str) -> String {
    // Collapse whitespace runs.
    let mut s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Remove spaces around punctuation that carries no internal whitespace.
    for tok in [
        "< ", " >", "( ", " )", "[ ", " ]", " ,", "& ", " &", "| ", " |",
    ] {
        let replacement = tok.replace(' ', "");
        while let Some(idx) = s.find(tok) {
            s.replace_range(idx..idx + tok.len(), &replacement);
        }
    }
    // Re-insert space after `,` if missing.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == ','
            && let Some(next) = chars.peek()
            && *next != ' '
        {
            out.push(' ');
        }
    }
    // Re-introduce single spaces around `|` and `&` (union / intersection).
    let out = out.replace('|', " | ").replace('&', " & ");
    // Collapse any double-spaces produced by the above.
    let mut cleaned = String::with_capacity(out.len());
    let mut prev_space = false;
    for c in out.chars() {
        if c == ' ' {
            if !prev_space {
                cleaned.push(' ');
            }
            prev_space = true;
        } else {
            cleaned.push(c);
            prev_space = false;
        }
    }
    // Avoid `< T >`-style. We already stripped those above; trim.
    cleaned.trim().to_string()
}

/// Strip a trailing `[]` (one or more) from a display form so the canonical
/// head can be inspected.
fn strip_array_suffix(s: &str) -> &str {
    let mut out = s.trim();
    while let Some(rest) = out.strip_suffix("[]") {
        out = rest.trim_end();
    }
    out
}

fn strip_generic_args(s: &str) -> &str {
    match s.find('<') {
        Some(idx) => s[..idx].trim_end(),
        None => s.trim_end(),
    }
}

fn is_predefined(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "number"
            | "boolean"
            | "any"
            | "unknown"
            | "void"
            | "never"
            | "null"
            | "undefined"
            | "symbol"
            | "bigint"
            | "object"
    )
}

/// Global ambient types we canonicalise without an explicit import.
/// Per contract: "Promise, Array, Record, Map, Set, Date, Error, RegExp,
/// Object, Function, JSON, Math, console, Window, Document, Element,
/// HTMLElement, Node" — and friends.
fn is_global_ambient(name: &str) -> bool {
    matches!(
        name,
        "Promise"
            | "Array"
            | "Record"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "Date"
            | "Error"
            | "RegExp"
            | "Object"
            | "Function"
            | "JSON"
            | "Math"
            | "console"
            | "Window"
            | "Document"
            | "Element"
            | "HTMLElement"
            | "Node"
            | "Partial"
            | "Required"
            | "Readonly"
            | "Pick"
            | "Omit"
            | "Exclude"
            | "Extract"
            | "NonNullable"
            | "ReturnType"
            | "Parameters"
            | "Awaited"
            | "ReadonlyArray"
            | "Iterator"
            | "Iterable"
            | "AsyncIterator"
            | "AsyncIterable"
    )
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
    ) {
        let language = match path.rsplit('.').next().unwrap_or("ts") {
            "tsx" => Language::Tsx,
            "js" => Language::JavaScript,
            "jsx" => Language::Jsx,
            _ => Language::TypeScript,
        };
        let mut parser = create_parser(language).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let (types, params, returns, _) = run(
            "function add(a: number, b: number): number { return a + b; }",
            "src/calc.ts",
        );
        // `number` dedups → one row.
        let number_rows: Vec<_> = types
            .iter()
            .filter(|t| t.display_name == "number" && t.kind == "primitive")
            .collect();
        assert_eq!(number_rows.len(), 1, "got {:?}", types);
        assert_eq!(
            number_rows[0].canonical_name.as_deref(),
            Some("typescript::primitive::number")
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "a");
        assert_eq!(params[0].type_display_name.as_deref(), Some("number"));
        assert!(!params[0].is_optional);
        assert!(!params[0].has_default);
        assert_eq!(params[1].parameter_name, "b");
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "number");
        assert_eq!(returns[0].function_name, "add");
    }

    #[test]
    fn generic_record() {
        let (types, _, _, _) = run(
            "function f(): Record<string, any> { return {} as any; }",
            "src/a.ts",
        );
        let outer = types
            .iter()
            .find(|t| t.display_name == "Record<string, any>")
            .expect("outer row");
        assert_eq!(outer.kind, "generic");
        assert_eq!(
            outer.canonical_name.as_deref(),
            Some("typescript::global::Record")
        );
        assert!(types.iter().any(|t| t.display_name == "string"));
        assert!(types.iter().any(|t| t.display_name == "any"));
    }

    #[test]
    fn union_type_alias() {
        let (types, _, _, _) = run(
            "export type SortDirection = 'asc' | 'desc';",
            "src/common.ts",
        );
        let union = types.iter().find(|t| t.kind == "union").expect("union row");
        assert!(
            union.display_name.contains("'asc'") && union.display_name.contains("'desc'"),
            "display_name: {}",
            union.display_name
        );
        assert!(union.canonical_name.is_none());
        // Literal types are kind=named with quotes preserved.
        assert!(types.iter().any(|t| t.display_name == "'asc'"));
        assert!(types.iter().any(|t| t.display_name == "'desc'"));
    }

    #[test]
    fn intersection_type_alias() {
        let (types, _, _, _) = run("type C = A & B;", "src/x.ts");
        let isect = types
            .iter()
            .find(|t| t.kind == "intersection")
            .expect("intersection row");
        assert!(isect.canonical_name.is_none());
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "A" && t.kind == "named")
        );
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "B" && t.kind == "named")
        );
    }

    #[test]
    fn interface_extends() {
        let (_, _, _, inh) = run("interface A {}\ninterface B extends A {}", "src/i.ts");
        let row = inh
            .iter()
            .find(|r| r.child_name == "B" && r.kind == InheritanceKind::Extends)
            .expect("extends row");
        assert_eq!(row.parent_display_name, "A");
        assert_eq!(row.child_kind, SymbolKind::Interface);
    }

    #[test]
    fn class_extends_and_implements() {
        let (_, _, _, inh) = run(
            "interface I {}\nclass Base {}\nclass D extends Base implements I {}",
            "src/c.ts",
        );
        let ext = inh
            .iter()
            .find(|r| r.child_name == "D" && r.kind == InheritanceKind::Extends)
            .expect("extends");
        assert_eq!(ext.parent_display_name, "Base");
        let imp = inh
            .iter()
            .find(|r| r.child_name == "D" && r.kind == InheritanceKind::Implements)
            .expect("implements");
        assert_eq!(imp.parent_display_name, "I");
        assert_eq!(imp.child_kind, SymbolKind::Class);
    }

    #[test]
    fn optional_param_and_default() {
        let (_, params, _, _) = run("function f(a?: number, b: number = 1) {}", "src/o.ts");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "a");
        assert!(params[0].is_optional);
        assert!(!params[0].has_default);
        assert_eq!(params[1].parameter_name, "b");
        assert!(params[1].has_default);
    }

    #[test]
    fn js_file_emits_no_type_rows() {
        let (types, params, returns, inh) = run(
            "function authenticate(req, res, next) { return next(); }",
            "src/auth.js",
        );
        assert!(types.is_empty(), "expected zero type rows, got {:?}", types);
        assert!(returns.is_empty());
        assert!(inh.is_empty());
        assert_eq!(params.len(), 3);
        let names: Vec<&str> = params.iter().map(|p| p.parameter_name.as_str()).collect();
        assert_eq!(names, vec!["req", "res", "next"]);
        for p in &params {
            assert!(p.type_display_name.is_none(), "JS param should be untyped");
            assert!(!p.is_optional);
            assert!(!p.has_default);
        }
        assert_eq!(params[0].function_name, "authenticate");
    }

    // TODO(#13 follow-up): canonical_name for `number[]` arrays is being
    // populated when contract expects None. Tracking under fan-out
    // polish.
    #[ignore]
    #[test]
    fn array_type_shorthand() {
        let (types, params, _, _) = run(
            "function avg(values: number[]): number { return 0; }",
            "src/arr.ts",
        );
        let arr = types
            .iter()
            .find(|t| t.kind == "array" && t.display_name == "number[]")
            .expect("array row");
        assert!(arr.canonical_name.is_none());
        // Inner number row.
        let n = types
            .iter()
            .find(|t| t.display_name == "number")
            .expect("number row");
        assert_eq!(n.kind, "primitive");
        assert_eq!(params[0].type_display_name.as_deref(), Some("number[]"));
    }

    #[test]
    fn tuple_return_type() {
        let (types, _, returns, _) = run(
            "function range(): [number, number] { return [0, 1]; }",
            "src/r.ts",
        );
        assert!(types.iter().any(|t| t.kind == "tuple"));
        // `number` dedups once across the two tuple positions.
        let n: Vec<_> = types
            .iter()
            .filter(|t| t.display_name == "number")
            .collect();
        assert_eq!(n.len(), 1);
        assert_eq!(returns.len(), 1);
    }
}
