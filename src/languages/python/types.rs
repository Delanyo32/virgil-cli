//! Issue #13 Python — type-expression / signature / inheritance
//! extractor. Per ADR-0003 (Level 3): full kind decomposition +
//! canonical_name resolution. Contract: docs/types-python.md.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::models::{
    ExtractedTypes, FieldTypeRow, InheritanceKind, InheritanceRow, ParameterTypeRow,
    ReturnsTypeRow, SymbolKind, TypeRow,
};

pub fn extract_types(tree: &Tree, source: &[u8], file_path: &str) -> ExtractedTypes {
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
    /// `(local_name, canonical_module_path)` pairs from file-scope
    /// `import X` / `from M import N [as L]` statements.
    use_bindings: Vec<UseBinding>,
    /// Same-file top-level definitions (`class`/`def`/module var) that
    /// resolve to `<module>.<name>`.
    same_file_defs: HashSet<String>,
    /// Module dotted path derived from the file path
    /// (`app/models.py` → `app.models`).
    module_path: String,
}

struct UseBinding {
    local_name: String,
    /// Fully qualified `<module>.<name>` (or just `<module>` for
    /// `import mod`).
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
            module_path: derive_module_path(file_path),
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

    /// Pre-pass: collect file-scope imports + top-level definitions so
    /// the main walk can resolve `canonical_name`.
    fn collect_file_level(&mut self, root: Node) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "import_statement" => self.collect_import_statement(child),
                "import_from_statement" => self.collect_import_from(child),
                "function_definition" | "class_definition" => {
                    if let Some(name) = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                    {
                        self.same_file_defs.insert(name.to_string());
                    }
                }
                "decorated_definition" => {
                    if let Some(inner) = child.child_by_field_name("definition")
                        && let Some(name) = inner
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(self.source).ok())
                    {
                        self.same_file_defs.insert(name.to_string());
                    }
                }
                "expression_statement" => {
                    let mut c = child.walk();
                    for inner in child.named_children(&mut c) {
                        if inner.kind() == "assignment"
                            && let Some(left) = inner.child_by_field_name("left")
                            && left.kind() == "identifier"
                            && let Ok(text) = left.utf8_text(self.source)
                        {
                            self.same_file_defs.insert(text.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_import_statement(&mut self, node: Node) {
        // `import a`, `import a.b`, `import a as b`, `import a, b`
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "dotted_name" | "identifier" => {
                    let text = child
                        .utf8_text(self.source)
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if text.is_empty() {
                        continue;
                    }
                    // Local name is the leading segment, canonical is the
                    // full dotted path (we'll treat the binding as the
                    // module itself; attribute access resolves via the
                    // dotted name).
                    let local = text.split('.').next().unwrap_or(&text).to_string();
                    self.use_bindings.push(UseBinding {
                        local_name: local,
                        canonical_path: text,
                    });
                }
                "aliased_import" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let alias = child
                        .child_by_field_name("alias")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if !name.is_empty() && !alias.is_empty() {
                        self.use_bindings.push(UseBinding {
                            local_name: alias,
                            canonical_path: name,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_import_from(&mut self, node: Node) {
        // Module name lives between `from` and `import` keywords.
        let module = extract_from_module(node, self.source);
        if module.is_empty() {
            return;
        }
        let mut cursor = node.walk();
        let mut past_import = false;
        for child in node.children(&mut cursor) {
            if child.kind() == "import" {
                past_import = true;
                continue;
            }
            if !past_import {
                continue;
            }
            match child.kind() {
                "dotted_name" | "identifier" => {
                    let name = child
                        .utf8_text(self.source)
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let local = name.split('.').next().unwrap_or(&name).to_string();
                    let canonical = format!("{}.{}", module, name);
                    self.use_bindings.push(UseBinding {
                        local_name: local,
                        canonical_path: canonical,
                    });
                }
                "aliased_import" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let alias = child
                        .child_by_field_name("alias")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if !name.is_empty() && !alias.is_empty() {
                        self.use_bindings.push(UseBinding {
                            local_name: alias,
                            canonical_path: format!("{}.{}", module, name),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// Main walk over the tree.
    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => self.visit_function(node),
            "class_definition" => self.visit_class(node),
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
        }
    }

    fn visit_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(name) = name_node.utf8_text(self.source) else {
            return;
        };
        let kind = if is_inside_class(node) {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let (fn_line, fn_col) = node_pos(name_node);

        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, name, kind, fn_line, fn_col);
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            // `return_type` field on `function_definition` points at a
            // `type` node. Emit and record.
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
            let (pname, type_node, has_default, default_is_none) = match p.kind() {
                "identifier" => {
                    // bare positional: `x`
                    let name = p.utf8_text(self.source).unwrap_or("").to_string();
                    (name, None, false, false)
                }
                "typed_parameter" => {
                    // `x: T` — first named child is the bound identifier,
                    // `type` field holds the annotation node.
                    let name = first_param_name(p, self.source);
                    let t = p.child_by_field_name("type");
                    (name, t, false, false)
                }
                "default_parameter" => {
                    // `x = expr`
                    let name = p
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let is_none = p
                        .child_by_field_name("value")
                        .map(|v| is_none_literal(v, self.source))
                        .unwrap_or(false);
                    (name, None, true, is_none)
                }
                "typed_default_parameter" => {
                    // `x: T = expr`
                    let name = p
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let t = p.child_by_field_name("type");
                    let is_none = p
                        .child_by_field_name("value")
                        .map(|v| is_none_literal(v, self.source))
                        .unwrap_or(false);
                    (name, t, true, is_none)
                }
                "list_splat_pattern" => {
                    // `*args`
                    let name = first_param_name(p, self.source);
                    (name, None, false, false)
                }
                "dictionary_splat_pattern" => {
                    // `**kwargs`
                    let name = first_param_name(p, self.source);
                    (name, None, false, false)
                }
                _ => continue,
            };

            let (pl, pc) = node_pos(p);
            let type_display = if let Some(t) = type_node {
                self.emit_type_with_subtree(t);
                Some(render_type(t, self.source))
            } else {
                None
            };

            // is_optional: per contract Example 3 — true when either the
            // annotation is Optional[...]/X|None or the default is None.
            let is_optional = type_display
                .as_deref()
                .map(annotation_is_optional)
                .unwrap_or(false)
                || default_is_none;

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
            position += 1;
        }
    }

    /// `class Foo(Bar, Baz, metaclass=M): ...` — emit `extends` rows for
    /// each non-keyword positional base.
    fn visit_class(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // Inheritance: superclasses → extends rows.
        if let Some(supers) = node.child_by_field_name("superclasses") {
            let mut cursor = supers.walk();
            for base in supers.named_children(&mut cursor) {
                // The `superclasses` node is an `argument_list`; skip
                // `keyword_argument` (e.g. `metaclass=`) and emit one
                // row per positional base.
                if base.kind() == "keyword_argument" {
                    continue;
                }
                let display = render_type(base, self.source);
                if display.is_empty() {
                    continue;
                }
                self.emit_type_with_subtree(base);
                let canonical = self.resolve_head(&display);
                self.inheritance.push(InheritanceRow {
                    file_path: self.file_path.to_string(),
                    child_start_line: cl,
                    child_start_col: cc,
                    child_name: child_name.to_string(),
                    child_kind: SymbolKind::Class,
                    parent_display_name: display,
                    parent_canonical_name: canonical,
                    kind: InheritanceKind::Extends,
                });
            }
        }

        // Issue #14: PEP 526 typed class attributes. Tree-sitter-python
        // exposes `x: int = 5` as an `expression_statement` containing
        // an `assignment` with a `type` field. Untyped attributes
        // (`x = 5`) have no `type` and emit no row.
        if let Some(body) = node.child_by_field_name("body") {
            let mut c = body.walk();
            for stmt in body.named_children(&mut c) {
                if stmt.kind() != "expression_statement" {
                    continue;
                }
                let mut sc = stmt.walk();
                for inner in stmt.named_children(&mut sc) {
                    if inner.kind() != "assignment" {
                        continue;
                    }
                    let Some(t) = inner.child_by_field_name("type") else {
                        continue;
                    };
                    let Some(left) = inner.child_by_field_name("left") else {
                        continue;
                    };
                    if left.kind() != "identifier" {
                        continue;
                    }
                    self.emit_type_with_subtree(t);
                    if let Ok(field_name) = left.utf8_text(self.source) {
                        let (line, col) = node_pos(left);
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
        }
    }

    /// Emit a TypeRow for `node` (a `type` wrapper or a sub-type
    /// expression) and recursively for every nested type argument.
    fn emit_type_with_subtree(&mut self, node: Node) {
        // Unwrap the grammar's `type` wrapper to its first named child for
        // classification, but use the wrapper for `display_name` (the
        // wrapper's text equals its inner expression).
        let inner = if node.kind() == "type" {
            node.named_child(0).unwrap_or(node)
        } else {
            node
        };

        if let Some((kind, display)) = self.classify_type_node(inner) {
            let canonical = self.resolve_head_for_kind(&display, &kind, inner);
            if self.seen_display.insert(display.clone()) {
                self.types.push(TypeRow {
                    file_path: self.file_path.to_string(),
                    kind,
                    display_name: display,
                    canonical_name: canonical,
                });
            }
        }

        // Recurse into the argument list of generic / subscript types so
        // each arg gets its own row. PEP 604 unions (binary_operator) are
        // also walked so each operand becomes a row.
        let mut cursor = inner.walk();
        for c in inner.named_children(&mut cursor) {
            match (inner.kind(), c.kind()) {
                ("generic_type", "type_parameter") => {
                    let mut tc = c.walk();
                    for arg in c.named_children(&mut tc) {
                        if arg.kind() == "type" {
                            self.emit_type_with_subtree(arg);
                        } else if arg.kind() == "list" {
                            // Callable[[A, B], R] — first arg is a list
                            // of types; walk those too.
                            let mut lc = arg.walk();
                            for inner_arg in arg.named_children(&mut lc) {
                                if inner_arg.kind() == "type" {
                                    self.emit_type_with_subtree(inner_arg);
                                }
                            }
                        }
                    }
                }
                ("subscript", _)
                    // older grammars: `value` + `subscript` children
                    if c.kind() == "type" => {
                        self.emit_type_with_subtree(c);
                    }
                ("binary_operator", _) => {
                    // PEP 604 — operands are themselves `type` nodes or
                    // bare identifiers.
                    if matches!(
                        c.kind(),
                        "type" | "identifier" | "attribute" | "none" | "binary_operator"
                    ) {
                        self.emit_type_with_subtree(c);
                    }
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
            "identifier" | "none" => {
                if is_python_primitive(&display) {
                    "primitive"
                } else {
                    "named"
                }
            }
            "attribute" => "named",
            "string" => "named",
            "generic_type" | "subscript" => {
                // Look at the base name (first named child) to decide
                // override.
                let base = node.named_child(0);
                let base_text = base
                    .and_then(|b| b.utf8_text(self.source).ok())
                    .unwrap_or("");
                let bare = base_text.rsplit('.').next().unwrap_or(base_text);
                match bare {
                    "Optional" | "Union" => "union",
                    "Callable" => "function",
                    "Tuple" | "tuple" => "tuple",
                    "Literal" => "named",
                    _ => "generic",
                }
            }
            "binary_operator" => {
                // Only `|` chains are unions; other binary operators
                // shouldn't appear in type position. Be lenient: treat
                // any binary_operator inside a type as union.
                "union"
            }
            "tuple" => "tuple",
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    /// Per-kind canonical resolution. Wrappers (`union`/`function`/
    /// `tuple`/`generic` with a typing base) resolve to the wrapper's
    /// canonical `<typing>.X` or `<builtin>.X`; PEP 604 `|` unions
    /// resolve to `null`.
    fn resolve_head_for_kind(&self, display: &str, kind: &str, node: Node) -> Option<String> {
        if kind == "union" && node.kind() == "binary_operator" {
            return None;
        }
        if matches!(kind, "union" | "function" | "tuple" | "generic" | "named")
            && matches!(node.kind(), "generic_type" | "subscript")
        {
            // Resolve via the wrapper's base name.
            let base = node.named_child(0);
            let base_text = base
                .and_then(|b| b.utf8_text(self.source).ok())
                .unwrap_or("");
            return self.resolve_head(base_text);
        }
        self.resolve_head(display)
    }

    /// Resolve a bare/dotted name to its canonical path per the rules in
    /// docs/types-python.md. Returns `None` when unresolvable.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = strip_quotes(display.trim());
        // Strip generic args off for the head segment.
        let head = strip_generic_args(head);
        let head = head.trim();
        if head.is_empty() || head == "_" {
            return None;
        }

        let first_segment = head.split('.').next().unwrap_or(head);

        // 1. Primitives → <builtin>.<name> (or <typing>.Any).
        if is_python_primitive(first_segment) && head == first_segment {
            return Some(builtin_canonical(first_segment));
        }

        // 1b. Builtin container/collection types referenced bare —
        // `list`, `dict`, `set`, `tuple`, `frozenset` (PEP 585).
        if is_builtin_container(first_segment) && head == first_segment {
            return Some(format!("<builtin>.{}", first_segment));
        }

        // 2. Drop `typing.` prefix for typing names.
        if let Some(rest) = head.strip_prefix("typing.")
            && is_typing_name(rest)
        {
            return Some(format!("<typing>.{}", rest));
        }
        if is_typing_name(head) {
            return Some(format!("<typing>.{}", head));
        }

        // 3. Imported binding match.
        for u in &self.use_bindings {
            if u.local_name == first_segment {
                if head == first_segment {
                    return Some(u.canonical_path.clone());
                }
                let rest = &head[first_segment.len()..]; // includes leading '.'
                return Some(format!("{}{}", u.canonical_path, rest));
            }
        }

        // 4. Same-file top-level def → module.name.
        if head == first_segment && self.same_file_defs.contains(first_segment) {
            return Some(format!("{}.{}", self.module_path, first_segment));
        }

        None
    }
}

/// `app/models.py` → `app.models`; `pkg/__init__.py` → `pkg`.
fn derive_module_path(file_path: &str) -> String {
    let path = file_path.trim_start_matches("./");
    let path = path.strip_suffix(".py").unwrap_or(path);
    let path = path.strip_suffix("/__init__").unwrap_or(path);
    path.replace('/', ".")
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

fn is_inside_class(node: Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_definition" => return true,
            "function_definition" => return false,
            _ => current = parent.parent(),
        }
    }
    false
}

fn first_param_name(p: Node, source: &[u8]) -> String {
    let mut cursor = p.walk();
    for child in p.named_children(&mut cursor) {
        if child.kind() == "identifier"
            && let Ok(text) = child.utf8_text(source)
        {
            return text.to_string();
        }
    }
    String::new()
}

fn is_none_literal(node: Node, source: &[u8]) -> bool {
    if node.kind() == "none" {
        return true;
    }
    node.utf8_text(source).unwrap_or("").trim() == "None"
}

fn render_type(node: Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    normalize_type_text(text)
}

fn normalize_type_text(raw: &str) -> String {
    // Strip surrounding quotes (string forward reference like `"User"`).
    let trimmed = raw.trim();
    let unquoted = if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    // Collapse whitespace runs.
    let mut s: String = unquoted.split_whitespace().collect::<Vec<_>>().join(" ");

    // Remove spaces around `[`, `]`, `(`, `)`.
    for tok in ["[ ", " ]", "( ", " )"] {
        let replacement = tok.replace(' ', "");
        while let Some(idx) = s.find(tok) {
            s.replace_range(idx..idx + tok.len(), &replacement);
        }
    }

    // Drop `typing.` prefix for well-known typing names (rule 3).
    // We only strip when the prefix appears at the start of a token —
    // walk char-by-char.
    let s = strip_typing_prefixes(&s);

    // Re-insert space after `,` if missing, and exactly one space around `|`.
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '|' {
            // ensure single space before
            if !out.ends_with(' ') {
                out.push(' ');
            }
            out.push('|');
            // ensure single space after
            if i + 1 < chars.len() && chars[i + 1] != ' ' {
                out.push(' ');
            }
            i += 1;
            continue;
        }
        if c == ',' {
            out.push(',');
            if i + 1 < chars.len() {
                let next = chars[i + 1];
                if next != ' ' && next != ']' && next != ')' {
                    out.push(' ');
                }
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }

    // Drop trailing commas inside generic args: `Tuple[int, str,]` → `Tuple[int, str]`.
    let mut cleaned = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ',' {
            // peek past spaces
            let mut j = i + 1;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if j < chars.len() && (chars[j] == ']' || chars[j] == ')') {
                i = j;
                continue;
            }
        }
        cleaned.push(c);
        i += 1;
    }

    cleaned.trim().to_string()
}

fn strip_typing_prefixes(s: &str) -> String {
    // Replace `typing.X` with `X` when X is a well-known typing name.
    // Python type-position source is ASCII in practice, so a byte-level
    // scan is safe.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || !is_ident_byte(bytes[i - 1]);
        if at_boundary && bytes.len() - i >= 7 && &bytes[i..i + 7] == b"typing." {
            let after = &s[i + 7..];
            let end = after
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            let name = &after[..end];
            if is_typing_name(name) {
                out.push_str(name);
                i += 7 + name.len();
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn strip_quotes(s: &str) -> &str {
    let t = s.trim();
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

fn strip_generic_args(s: &str) -> &str {
    match s.find('[') {
        Some(idx) => &s[..idx],
        None => s,
    }
}

/// Per contract: primitive scalar types + `Any` + `None`/`NoneType` +
/// `object`.
fn is_python_primitive(name: &str) -> bool {
    matches!(
        name,
        "int"
            | "float"
            | "bool"
            | "str"
            | "bytes"
            | "bytearray"
            | "complex"
            | "None"
            | "NoneType"
            | "object"
            | "Any"
    )
}

fn is_builtin_container(name: &str) -> bool {
    matches!(
        name,
        "list" | "dict" | "set" | "tuple" | "frozenset" | "type"
    )
}

fn builtin_canonical(name: &str) -> String {
    match name {
        "Any" => "<typing>.Any".to_string(),
        _ => format!("<builtin>.{}", name),
    }
}

/// Names directly importable from `typing` per docs/types-python.md
/// rule 4.
fn is_typing_name(name: &str) -> bool {
    matches!(
        name,
        "Optional"
            | "Union"
            | "Callable"
            | "List"
            | "Dict"
            | "Tuple"
            | "Set"
            | "FrozenSet"
            | "Literal"
            | "Iterable"
            | "Iterator"
            | "Mapping"
            | "MutableMapping"
            | "Sequence"
            | "MutableSequence"
            | "Generator"
            | "AsyncGenerator"
            | "Awaitable"
            | "Coroutine"
            | "Type"
            | "Any"
            | "ClassVar"
            | "Final"
            | "Annotated"
            | "TypedDict"
            | "Protocol"
            | "NoReturn"
            | "Never"
    )
}

/// `Optional[T]` or `T | None` → annotation is optional.
fn annotation_is_optional(display: &str) -> bool {
    let d = display.trim();
    if d.starts_with("Optional[") {
        return true;
    }
    // PEP 604: any `| None` operand (with surrounding spaces per
    // normalization).
    d == "None" || d.ends_with(" | None") || d.starts_with("None | ") || d.contains(" | None | ")
}

/// Extract the module text from a `from ... import ...` node (handles
/// relative imports too, e.g. `..pkg`).
fn extract_from_module(node: Node, source: &[u8]) -> String {
    if let Some(m) = node.child_by_field_name("module_name") {
        return m.utf8_text(source).unwrap_or("").trim().to_string();
    }
    let mut cursor = node.walk();
    let mut found_from = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "from" {
            found_from = true;
            continue;
        }
        if found_from && child.kind() == "import" {
            break;
        }
        if found_from {
            match child.kind() {
                "dotted_name" | "relative_import" => {
                    return child.utf8_text(source).unwrap_or("").trim().to_string();
                }
                _ => {}
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;

    fn run(source: &str, path: &str) -> ExtractedTypes {
        let mut parser = create_parser(Language::Python).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn typed_param_and_return() {
        let (types, params, returns, _, _) =
            run("def f(x: int) -> str:\n    return str(x)\n", "app/m.py");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "int" && t.kind == "primitive")
        );
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "str" && t.kind == "primitive")
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].parameter_name, "x");
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert_eq!(params[0].position, 0);
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "str");
    }

    #[test]
    fn class_with_bases() {
        let (_, _, _, inh, _) = run("class Child(Base, Mixin):\n    pass\n", "app/m.py");
        let parents: Vec<&str> = inh
            .iter()
            .filter(|r| r.child_name == "Child" && r.kind == InheritanceKind::Extends)
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert!(parents.contains(&"Base"), "missing Base in {:?}", parents);
        assert!(parents.contains(&"Mixin"), "missing Mixin in {:?}", parents);
    }

    #[test]
    fn untyped_param_emits_none_type() {
        let (types, params, returns, _, _) = run("def chunks(lst, n):\n    pass\n", "app/utils.py");
        assert!(types.is_empty(), "no type rows for unannotated fn");
        assert_eq!(params.len(), 2);
        assert!(params.iter().all(|p| p.type_display_name.is_none()));
        assert!(returns.is_empty());
    }

    #[test]
    fn union_pep604() {
        let (types, _, returns, _, _) = run(
            "def f(x: int) -> str | None:\n    return None\n",
            "app/m.py",
        );
        let row = types
            .iter()
            .find(|t| t.display_name == "str | None")
            .expect("union row");
        assert_eq!(row.kind, "union");
        assert!(
            row.canonical_name.is_none(),
            "PEP 604 union canonical = null"
        );
        assert_eq!(returns[0].type_display_name, "str | None");
        // Operands should each get their own row.
        assert!(types.iter().any(|t| t.display_name == "str"));
        assert!(types.iter().any(|t| t.display_name == "None"));
    }

    #[test]
    fn optional_param_is_union() {
        let (types, params, _, _, _) = run(
            "from typing import Optional\ndef f(id: Optional[int] = None):\n    pass\n",
            "app/m.py",
        );
        let row = types
            .iter()
            .find(|t| t.display_name == "Optional[int]")
            .expect("optional row");
        assert_eq!(row.kind, "union");
        assert_eq!(row.canonical_name.as_deref(), Some("<typing>.Optional"));
        // is_optional must reflect Optional[...] AND default=None.
        let p = params.iter().find(|p| p.parameter_name == "id").unwrap();
        assert!(p.is_optional);
        assert!(p.has_default);
    }

    #[test]
    fn class_typed_attribute_emits_field_type_row() {
        let (_, _, _, _, fields) = run("class C:\n    x: int = 5\n    y = 6\n", "app/m.py");
        assert_eq!(fields.len(), 1, "got {fields:?}");
        assert_eq!(fields[0].field_name, "x");
        assert_eq!(fields[0].type_display_name, "int");
        assert_eq!(fields[0].field_kind, SymbolKind::Field);
    }

    #[test]
    fn list_int_generic() {
        let (types, params, _, _, _) = run("def f(xs: list[int]):\n    pass\n", "app/m.py");
        let row = types
            .iter()
            .find(|t| t.display_name == "list[int]")
            .expect("generic row");
        assert_eq!(row.kind, "generic");
        // Inner `int` row must exist independently.
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "int" && t.kind == "primitive")
        );
        assert_eq!(params[0].type_display_name.as_deref(), Some("list[int]"));
    }
}
