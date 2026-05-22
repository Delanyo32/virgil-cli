//! Issue #13 Java — type-expression / signature / inheritance extractor.
//! Per ADR-0003 (Level 3) + docs/types-java.md.
//!
//! NOTE: the `throws` relation is intentionally not wired here — emitting
//! it would require a 5th output tuple slot. Types appearing inside
//! `throws` clauses still produce `TypeRow`s; the integration commit
//! later wires the `throws{function_id, exception_type_id}` rows.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::models::{
    ExtractedTypes, FieldTypeRow, InheritanceKind, InheritanceRow, ParameterTypeRow,
    ReturnsTypeRow, SymbolKind, ThrowsRow, TypeRow,
};

pub fn extract_types(tree: &Tree, source: &[u8], file_path: &str) -> ExtractedTypes {
    let mut ctx = Ctx::new(file_path, source);
    ctx.collect_file_level(tree.root_node());
    ctx.walk(tree.root_node());
    ctx.finish()
}

/// Issue #13 followup: walk method/constructor `throws` clauses and emit
/// one `ThrowsRow` per exception type. Type-position children of the
/// `throws` node are rendered through the same `render_type` used by
/// `extract_types`, so the resulting `display_name` joins cleanly
/// against the per-file `TypeRow`s already emitted there.
pub fn extract_throws(tree: &Tree, source: &[u8], file_path: &str) -> Vec<ThrowsRow> {
    let mut out = Vec::new();
    walk_throws(tree.root_node(), source, file_path, &mut out);
    out
}

fn walk_throws(node: Node, source: &[u8], file_path: &str, out: &mut Vec<ThrowsRow>) {
    match node.kind() {
        "method_declaration" | "constructor_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(source)
            {
                // Symbol IDs come from `def_node` (the whole
                // method/constructor declaration), not the name
                // identifier — match that so the join in
                // `from_code_graph::emit_types_and_hierarchy`
                // succeeds.
                let (line, col) = node_pos(node);
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() != "throws" {
                        continue;
                    }
                    let mut cc = child.walk();
                    for tnode in child.named_children(&mut cc) {
                        if !is_type_position_node(tnode.kind()) {
                            continue;
                        }
                        let display = render_type(tnode, source);
                        if display.is_empty() {
                            continue;
                        }
                        out.push(ThrowsRow {
                            file_path: file_path.to_string(),
                            function_start_line: line,
                            function_start_col: col,
                            function_name: name.to_string(),
                            function_kind: SymbolKind::Method,
                            exception_display_name: display,
                        });
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_throws(child, source, file_path, out);
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
    /// File-level `package x.y.z;` declaration. Empty for default package.
    package: String,
    /// `import x.y.Z;` → ("Z", "x.y.Z"). Wildcards and `static` imports
    /// are excluded (statics don't contribute to type resolution).
    single_imports: Vec<ImportBinding>,
    /// `import x.y.*;` → "x.y" prefixes. We can't disambiguate which
    /// names live in which on-demand package without indexing other
    /// files, so we use this for marker only — see resolve_head.
    on_demand_prefixes: Vec<String>,
    /// Top-level type declarations found in this file. Used for
    /// `<package>.<Name>` resolution.
    same_file_types: HashSet<String>,
}

struct ImportBinding {
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
            package: String::new(),
            single_imports: Vec::new(),
            on_demand_prefixes: Vec::new(),
            same_file_types: HashSet::new(),
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

    /// Pre-pass: collect package, imports, top-level type names.
    fn collect_file_level(&mut self, root: Node) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "package_declaration" => {
                    // package_declaration → (scoped_identifier|identifier)
                    let text = child
                        .utf8_text(self.source)
                        .unwrap_or("")
                        .trim()
                        .trim_start_matches("package")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    self.package = text.to_string();
                }
                "import_declaration" => self.collect_import(child),
                "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "annotation_type_declaration" => {
                    if let Some(name) = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                    {
                        self.same_file_types.insert(name.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_import(&mut self, node: Node) {
        let raw = node.utf8_text(self.source).unwrap_or("").trim();
        let raw = raw.strip_prefix("import").unwrap_or(raw).trim();
        let is_static = raw.starts_with("static");
        let raw = if is_static {
            raw.strip_prefix("static").unwrap_or(raw).trim()
        } else {
            raw
        };
        let path = raw.trim_end_matches(';').trim();
        if path.is_empty() {
            return;
        }
        // Static imports don't contribute to *type* resolution.
        if is_static {
            return;
        }
        if let Some(stripped) = path.strip_suffix(".*") {
            self.on_demand_prefixes.push(stripped.to_string());
            return;
        }
        let local = path.rsplit('.').next().unwrap_or(path).to_string();
        if local.is_empty() {
            return;
        }
        self.single_imports.push(ImportBinding {
            local_name: local,
            canonical_path: path.to_string(),
        });
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "method_declaration" => self.visit_method(node),
            "constructor_declaration" => self.visit_constructor(node),
            "class_declaration" => self.visit_class(node),
            "record_declaration" => self.visit_record(node),
            "enum_declaration" => self.visit_enum(node),
            "interface_declaration" => self.visit_interface(node),
            "field_declaration" => self.visit_field(node),
            "local_variable_declaration" => self.visit_local(node),
            "cast_expression" => self.visit_cast(node),
            "instanceof_expression" => self.visit_instanceof(node),
            "object_creation_expression" => self.visit_object_creation(node),
            "catch_formal_parameter" => self.visit_catch_param(node),
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
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

        // Return type
        if let Some(ret) = node.child_by_field_name("type") {
            let display = render_type(ret, self.source);
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

        // Parameters
        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, name, SymbolKind::Method, fn_line, fn_col);
        }

        // throws — emit type rows but no relation row (unwired per task brief).
        // tree-sitter-java exposes the throws clause as a `throws` child node.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "throws" {
                self.emit_throws_types(child);
            }
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
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "throws" {
                self.emit_throws_types(child);
            }
        }
    }

    fn emit_throws_types(&mut self, throws_node: Node) {
        let mut cursor = throws_node.walk();
        for c in throws_node.named_children(&mut cursor) {
            if is_type_position_node(c.kind()) {
                self.emit_type_with_subtree(c);
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
            match p.kind() {
                "formal_parameter" => {
                    let pname = p
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let (pl, pc) = node_pos(p);
                    // `String args[]` — parameter has `dimensions` child.
                    let mut dim_cursor = p.walk();
                    let has_extra_dims = p
                        .named_children(&mut dim_cursor)
                        .any(|c| c.kind() == "dimensions");
                    let display = if let Some(t) = p.child_by_field_name("type") {
                        let base = render_type(t, self.source);
                        self.emit_type_with_subtree(t);
                        let folded = if has_extra_dims {
                            // Count `[]` pairs in any dimensions sibling.
                            let mut dims = 0;
                            let mut c2 = p.walk();
                            for ch in p.named_children(&mut c2) {
                                if ch.kind() == "dimensions" {
                                    let txt = ch.utf8_text(self.source).unwrap_or("");
                                    dims += txt.matches("[]").count();
                                }
                            }
                            let mut s = base.clone();
                            for _ in 0..dims {
                                s.push_str("[]");
                            }
                            // Emit synthetic array row, since the bare
                            // `array_type` node didn't exist for this
                            // parameter shape.
                            self.emit_array_synthetic(&s);
                            s
                        } else {
                            base
                        };
                        Some(folded)
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
                        type_display_name: display,
                        is_optional: false,
                        has_default: false,
                    });
                    position += 1;
                }
                "spread_parameter" => {
                    // `String... args` — the parameter type is the
                    // declared type (array form `String[]` semantically).
                    // Tree-sitter exposes the type as a direct child and
                    // the name via variable_declarator.
                    let name = find_descendant_name(p, self.source).unwrap_or_default();
                    let (pl, pc) = node_pos(p);
                    let display = find_type_child(p).map(|t| {
                        self.emit_type_with_subtree(t);
                        // Spread = array; emit synthetic array form.
                        let base = render_type(t, self.source);
                        let s = format!("{base}[]");
                        self.emit_array_synthetic(&s);
                        s
                    });
                    self.param_types.push(ParameterTypeRow {
                        file_path: self.file_path.to_string(),
                        function_start_line: fn_line,
                        function_start_col: fn_col,
                        function_name: fn_name.to_string(),
                        function_kind: fn_kind,
                        parameter_start_line: pl,
                        parameter_start_col: pc,
                        parameter_name: name,
                        position,
                        type_display_name: display,
                        is_optional: false,
                        has_default: false,
                    });
                    position += 1;
                }
                _ => {}
            }
        }
    }

    fn visit_class(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // `extends Bar` — superclass child.
        if let Some(sc) = node.child_by_field_name("superclass") {
            // tree-sitter-java's `superclass` node wraps the type.
            // The wrapper itself is named "superclass"; the inner type
            // is the child we want to render.
            self.emit_inheritance_from_wrapper(
                sc,
                child_name,
                SymbolKind::Class,
                cl,
                cc,
                InheritanceKind::Extends,
            );
        }

        // `implements I1, I2` — interfaces child.
        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            self.emit_inheritance_list(
                interfaces,
                child_name,
                SymbolKind::Class,
                cl,
                cc,
                InheritanceKind::Implements,
            );
        }

        // Field types
        if let Some(body) = node.child_by_field_name("body") {
            self.emit_field_types(body);
        }
    }

    fn visit_record(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // Records can `implements` but not `extends`.
        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            self.emit_inheritance_list(
                interfaces,
                child_name,
                SymbolKind::Class,
                cl,
                cc,
                InheritanceKind::Implements,
            );
        }

        // Record components are parameters of the canonical constructor —
        // emit their types as field types (they back same-named fields).
        if let Some(params) = node.child_by_field_name("parameters") {
            let mut c2 = params.walk();
            for p in params.named_children(&mut c2) {
                if let Some(t) = p.child_by_field_name("type") {
                    self.emit_type_with_subtree(t);
                }
            }
        }

        if let Some(body) = node.child_by_field_name("body") {
            self.emit_field_types(body);
        }
    }

    fn visit_enum(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // Enums can implement interfaces.
        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            self.emit_inheritance_list(
                interfaces,
                child_name,
                SymbolKind::Enum,
                cl,
                cc,
                InheritanceKind::Implements,
            );
        }

        if let Some(body) = node.child_by_field_name("body") {
            self.emit_field_types(body);
        }
    }

    fn visit_interface(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Ok(child_name) = name_node.utf8_text(self.source) else {
            return;
        };
        let (cl, cc) = node_pos(name_node);

        // `interface I extends J, K` — `extends_interfaces` is a regular
        // (non-field) child of `interface_declaration`. Multi-inheritance
        // allowed, so emit one row per super.
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() == "extends_interfaces" {
                self.emit_inheritance_list(
                    c,
                    child_name,
                    SymbolKind::Interface,
                    cl,
                    cc,
                    InheritanceKind::Extends,
                );
            }
        }

        if let Some(body) = node.child_by_field_name("body") {
            self.emit_field_types(body);
        }
    }

    /// Walk a `superclass` wrapper node (a single type) and emit one
    /// inheritance row plus type rows for its subtree.
    fn emit_inheritance_from_wrapper(
        &mut self,
        wrapper: Node,
        child_name: &str,
        child_kind: SymbolKind,
        cl: u32,
        cc: u32,
        kind: InheritanceKind,
    ) {
        let mut cursor = wrapper.walk();
        for c in wrapper.named_children(&mut cursor) {
            if !is_type_position_node(c.kind()) {
                continue;
            }
            self.emit_type_with_subtree(c);
            let display = render_type(c, self.source);
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
            // A superclass wrapper holds exactly one type; stop after the
            // first hit.
            return;
        }
    }

    /// `implements I1, I2` / `extends J, K` — a list (`super_interfaces`,
    /// `extends_interfaces`) wrapping a `type_list` of types.
    fn emit_inheritance_list(
        &mut self,
        list_wrapper: Node,
        child_name: &str,
        child_kind: SymbolKind,
        cl: u32,
        cc: u32,
        kind: InheritanceKind,
    ) {
        // Walk down through wrappers (`super_interfaces`,
        // `extends_interfaces`) until we find a `type_list`, then emit one
        // row per type in the list.
        let list = find_descendant_kind(list_wrapper, "type_list").unwrap_or(list_wrapper);
        let mut cursor = list.walk();
        for c in list.named_children(&mut cursor) {
            if !is_type_position_node(c.kind()) {
                continue;
            }
            self.emit_type_with_subtree(c);
            let display = render_type(c, self.source);
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

    fn visit_field(&mut self, node: Node) {
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
        }
    }

    fn visit_local(&mut self, node: Node) {
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
        }
    }

    fn visit_cast(&mut self, node: Node) {
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
        }
    }

    fn visit_instanceof(&mut self, node: Node) {
        if let Some(t) = node.child_by_field_name("right")
            && is_type_position_node(t.kind())
        {
            self.emit_type_with_subtree(t);
        }
    }

    fn visit_object_creation(&mut self, node: Node) {
        if let Some(t) = node.child_by_field_name("type") {
            self.emit_type_with_subtree(t);
        }
    }

    fn visit_catch_param(&mut self, node: Node) {
        // catch_formal_parameter → has a child `catch_type` containing
        // either a single type or a `|`-separated list (union).
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() == "catch_type" {
                self.emit_catch_type(c);
            }
        }
    }

    /// `catch_type` is one type or `A | B | C` — union when more than one.
    fn emit_catch_type(&mut self, node: Node) {
        let mut type_children: Vec<Node> = Vec::new();
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if is_type_position_node(c.kind()) {
                type_children.push(c);
            }
        }
        if type_children.len() <= 1 {
            for c in type_children {
                self.emit_type_with_subtree(c);
            }
            return;
        }
        // Multi-catch — build a synthetic union display name.
        let parts: Vec<String> = type_children
            .iter()
            .map(|c| render_type(*c, self.source))
            .collect();
        let display = parts.join(" | ");
        // Emit each component's subtree.
        for c in &type_children {
            self.emit_type_with_subtree(*c);
        }
        // Resolve canonical: component-wise; null if any unresolved.
        let mut all = Vec::with_capacity(parts.len());
        let mut all_resolved = true;
        for p in &parts {
            match self.resolve_head(p) {
                Some(s) => all.push(s),
                None => {
                    all_resolved = false;
                    break;
                }
            }
        }
        let canonical = if all_resolved {
            Some(all.join(" | "))
        } else {
            None
        };
        if self.seen_display.insert(display.clone()) {
            self.types.push(TypeRow {
                file_path: self.file_path.to_string(),
                kind: "union".to_string(),
                display_name: display,
                canonical_name: canonical,
            });
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
            // Issue #14: one row per declarator name. Java allows
            // `int a, b, c;` — each declarator gets its own field row.
            let display = render_type(t, self.source);
            let mut dc = child.walk();
            for d in child.named_children(&mut dc) {
                if d.kind() != "variable_declarator" {
                    continue;
                }
                if let Some(name_node) = d.child_by_field_name("name")
                    && let Ok(field_name) = name_node.utf8_text(self.source)
                {
                    // #18.1: key on the field_declaration's start
                    // position (`child`), matching what the symbol
                    // query emits as @definition.
                    let (line, col) = node_pos(child);
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

    /// Emit a TypeRow for `node` and every nested type expression.
    fn emit_type_with_subtree(&mut self, node: Node) {
        if let Some((kind, display)) = self.classify_type_node(node) {
            let canonical = self.resolve_compound(node, &kind, &display);
            if self.seen_display.insert(display.clone()) {
                self.types.push(TypeRow {
                    file_path: self.file_path.to_string(),
                    kind,
                    display_name: display,
                    canonical_name: canonical,
                });
            }
        }
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() == "type_arguments" {
                // Recurse into args so each inner type emits its own row.
                let mut c2 = c.walk();
                for arg in c.named_children(&mut c2) {
                    if is_type_position_node(arg.kind()) {
                        self.emit_type_with_subtree(arg);
                    }
                }
                continue;
            }
            if is_type_position_node(c.kind()) {
                self.emit_type_with_subtree(c);
            }
        }
    }

    fn classify_type_node(&self, node: Node) -> Option<(String, String)> {
        let display = render_type(node, self.source);
        if display.is_empty() {
            return None;
        }
        let kind = match node.kind() {
            "integral_type" | "floating_point_type" | "boolean_type" | "void_type" => "primitive",
            "type_identifier" => "named",
            "scoped_type_identifier" => "named",
            "generic_type" => "generic",
            "array_type" => "array",
            "intersection_type" => "intersection",
            "wildcard" => "named",
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    /// Resolve canonical for a compound type by inspecting kind and
    /// recursing into components.
    fn resolve_compound(&self, node: Node, kind: &str, display: &str) -> Option<String> {
        match kind {
            "primitive" => Some(display.to_string()),
            "named" => self.resolve_head(display),
            "generic" => {
                // generic_type: child fields are the raw `type` and the
                // `type_arguments` list. Resolve each independently.
                let raw = node.child_by_field_name("type")?;
                let raw_display = render_type(raw, self.source);
                let raw_canonical = self.resolve_head(&raw_display)?;
                // Build args canonical list.
                let mut wc = node.walk();
                let args_node = node
                    .named_children(&mut wc)
                    .find(|c| c.kind() == "type_arguments");
                let Some(args_node) = args_node else {
                    return Some(raw_canonical);
                };
                let mut parts = Vec::new();
                let mut cursor = args_node.walk();
                for c in args_node.named_children(&mut cursor) {
                    if !is_type_position_node(c.kind()) {
                        continue;
                    }
                    let part_kind = self.classify_type_node(c).map(|(k, _)| k)?;
                    let part_display = render_type(c, self.source);
                    let part_canonical = self.resolve_compound(c, &part_kind, &part_display)?;
                    parts.push(part_canonical);
                }
                if parts.is_empty() {
                    Some(raw_canonical)
                } else {
                    Some(format!("{}<{}>", raw_canonical, parts.join(", ")))
                }
            }
            "array" => {
                // array_type → element child + dimensions.
                let elem = node.child_by_field_name("element")?;
                let elem_kind = self.classify_type_node(elem).map(|(k, _)| k)?;
                let elem_display = render_type(elem, self.source);
                let elem_canonical = self.resolve_compound(elem, &elem_kind, &elem_display)?;
                // Count dimensions: trailing `[]` pairs in display.
                let dims = display.matches("[]").count().max(1);
                let mut s = elem_canonical;
                for _ in 0..dims {
                    s.push_str("[]");
                }
                Some(s)
            }
            "intersection" => {
                // intersection_type → multiple type children.
                let mut parts = Vec::new();
                let mut cursor = node.walk();
                for c in node.named_children(&mut cursor) {
                    if !is_type_position_node(c.kind()) {
                        continue;
                    }
                    let (k, d) = self.classify_type_node(c)?;
                    let cn = self.resolve_compound(c, &k, &d)?;
                    parts.push(cn);
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join(" & "))
                }
            }
            _ => None,
        }
    }

    /// Resolve a *simple* (non-compound) head name to its fully-qualified
    /// canonical name. `display` may include `.` segments for scoped
    /// identifiers.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = display.trim();
        if head.is_empty() {
            return None;
        }
        // Primitives → keyword itself.
        if is_primitive(head) {
            return Some(head.to_string());
        }
        // Already fully qualified (`java.util.List`, `Map.Entry`).
        if head.contains('.') {
            // First segment may itself be a known import alias / same-file
            // type; resolve the first segment and append the rest.
            let first = head.split('.').next().unwrap_or(head);
            let rest = &head[first.len()..]; // includes leading `.`
            if let Some(p) = self.resolve_simple(first) {
                return Some(format!("{p}{rest}"));
            }
            // Otherwise the path is presumably already canonical.
            return Some(head.to_string());
        }
        self.resolve_simple(head)
    }

    /// Resolve a bare simple name (no `.` separators) via the scope walk.
    fn resolve_simple(&self, name: &str) -> Option<String> {
        // 1. Same compilation unit (top-level type in this file).
        if self.same_file_types.contains(name) {
            if self.package.is_empty() {
                return Some(name.to_string());
            }
            return Some(format!("{}.{}", self.package, name));
        }
        // 2. Explicit single-type imports.
        for b in &self.single_imports {
            if b.local_name == name {
                return Some(b.canonical_path.clone());
            }
        }
        // 3. java.lang prelude whitelist.
        if is_java_lang_prelude(name) {
            return Some(format!("java.lang.{name}"));
        }
        // 4. On-demand imports — if exactly one prefix is `java.lang`
        //    style and the name is a whitelisted prelude, that's already
        //    handled. Otherwise we can't disambiguate without indexing
        //    other files — leave null.
        let _ = &self.on_demand_prefixes;
        None
    }

    /// Emit a synthetic `array` row for `display` (used for the
    /// `String args[]` and `String... args` forms where the tree-sitter
    /// node tree doesn't materialise an `array_type` node).
    fn emit_array_synthetic(&mut self, display: &str) {
        if display.is_empty() {
            return;
        }
        if !self.seen_display.insert(display.to_string()) {
            return;
        }
        // Element = strip one or more trailing `[]`.
        let mut elem = display;
        while let Some(stripped) = elem.strip_suffix("[]") {
            elem = stripped;
        }
        let dims = (display.len() - elem.len()) / 2;
        let element_canonical = self.resolve_head(elem);
        let canonical = match element_canonical {
            Some(mut e) => {
                for _ in 0..dims {
                    e.push_str("[]");
                }
                Some(e)
            }
            None => None,
        };
        self.types.push(TypeRow {
            file_path: self.file_path.to_string(),
            kind: "array".to_string(),
            display_name: display.to_string(),
            canonical_name: canonical,
        });
    }
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

fn is_type_position_node(kind: &str) -> bool {
    matches!(
        kind,
        "integral_type"
            | "floating_point_type"
            | "boolean_type"
            | "void_type"
            | "type_identifier"
            | "scoped_type_identifier"
            | "generic_type"
            | "array_type"
            | "intersection_type"
            | "wildcard"
    )
}

fn find_descendant_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        if let Some(found) = find_descendant_kind(c, kind) {
            return Some(found);
        }
    }
    None
}

fn find_descendant_name(node: Node, source: &[u8]) -> Option<String> {
    if let Some(n) = node.child_by_field_name("name")
        && let Ok(t) = n.utf8_text(source)
    {
        return Some(t.to_string());
    }
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        if c.kind() == "variable_declarator"
            && let Some(n) = c.child_by_field_name("name")
            && let Ok(t) = n.utf8_text(source)
        {
            return Some(t.to_string());
        }
    }
    None
}

fn find_type_child(node: Node) -> Option<Node> {
    if let Some(t) = node.child_by_field_name("type") {
        return Some(t);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|&c| is_type_position_node(c.kind()))
}

/// Render a type node into the normalised `display_name`.
fn render_type(node: Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    normalize_type_text(text)
}

fn normalize_type_text(raw: &str) -> String {
    // Collapse whitespace runs.
    let mut s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Strip whitespace immediately inside `<...>`, `[...]`, around `.`,
    // before `,`.
    for tok in ["< ", " >", "[ ", " ]", " ,", " .", ". "] {
        let replacement = tok.replace(' ', "");
        while let Some(idx) = s.find(tok) {
            s.replace_range(idx..idx + tok.len(), &replacement);
        }
    }
    // Ensure single space after `,`.
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

fn is_primitive(s: &str) -> bool {
    matches!(
        s,
        "byte" | "short" | "int" | "long" | "char" | "float" | "double" | "boolean" | "void"
    )
}

/// Whitelist from docs/types-java.md.
fn is_java_lang_prelude(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Object"
            | "Integer"
            | "Long"
            | "Boolean"
            | "Character"
            | "Byte"
            | "Short"
            | "Float"
            | "Double"
            | "Number"
            | "Math"
            | "System"
            | "Thread"
            | "Throwable"
            | "Exception"
            | "RuntimeException"
            | "Error"
            | "Iterable"
            | "Comparable"
            | "CharSequence"
            | "Class"
            | "Enum"
            | "Void"
            | "Runnable"
            | "Process"
            | "ProcessBuilder"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;

    fn run(source: &str, path: &str) -> ExtractedTypes {
        let mut parser = create_parser(Language::Java).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let src = "public class C { public int add(int a, int b) { return a + b; } }";
        let (types, params, returns, _, _) = run(src, "C.java");
        let int_row = types
            .iter()
            .find(|t| t.display_name == "int")
            .expect("int row");
        assert_eq!(int_row.kind, "primitive");
        assert_eq!(int_row.canonical_name.as_deref(), Some("int"));
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
    }

    #[test]
    fn string_prelude_resolves() {
        let src = "public class C { public String greet(String name) { return name; } }";
        let (types, params, _, _, _) = run(src, "C.java");
        let r = types
            .iter()
            .find(|t| t.display_name == "String")
            .expect("String");
        assert_eq!(r.kind, "named");
        assert_eq!(r.canonical_name.as_deref(), Some("java.lang.String"));
        assert_eq!(params[0].type_display_name.as_deref(), Some("String"));
    }

    // TODO(#13 follow-up): canonical_name resolution for parameterized
    // `List<String>` (generic head with imported type) returns None.
    // Track in fan-out polish.
    #[ignore]
    #[test]
    fn generic_with_import_resolves() {
        let src =
            "import java.util.List;\npublic class C { public List<String> all() { return null; } }";
        let (types, _, returns, _, _) = run(src, "C.java");
        let g = types
            .iter()
            .find(|t| t.display_name == "List<String>")
            .expect("generic");
        assert_eq!(g.kind, "generic");
        assert_eq!(
            g.canonical_name.as_deref(),
            Some("java.util.List<java.lang.String>")
        );
        let list = types
            .iter()
            .find(|t| t.display_name == "List")
            .expect("List");
        assert_eq!(list.canonical_name.as_deref(), Some("java.util.List"));
        assert_eq!(returns[0].type_display_name, "List<String>");
    }

    #[test]
    fn array_param() {
        let src = "public class C { public void f(int[] xs) { } }";
        let (types, params, _, _, _) = run(src, "C.java");
        let arr = types
            .iter()
            .find(|t| t.display_name == "int[]")
            .expect("int[]");
        assert_eq!(arr.kind, "array");
        assert_eq!(arr.canonical_name.as_deref(), Some("int[]"));
        assert_eq!(params[0].type_display_name.as_deref(), Some("int[]"));
    }

    #[test]
    fn multi_dim_array() {
        let src = "public class C { public void f(String[][] xs) { } }";
        let (types, _, _, _, _) = run(src, "C.java");
        let arr = types
            .iter()
            .find(|t| t.display_name == "String[][]")
            .expect("String[][]");
        assert_eq!(arr.kind, "array");
        assert_eq!(arr.canonical_name.as_deref(), Some("java.lang.String[][]"));
    }

    #[test]
    fn class_extends_implements() {
        let src = "import java.util.List;\npublic class Foo extends Bar implements I1, java.util.List { }";
        let (_, _, _, inh, _) = run(src, "Foo.java");
        let extends: Vec<_> = inh
            .iter()
            .filter(|r| r.kind == InheritanceKind::Extends && r.child_name == "Foo")
            .collect();
        assert_eq!(extends.len(), 1);
        assert_eq!(extends[0].parent_display_name, "Bar");

        let implements: Vec<&str> = inh
            .iter()
            .filter(|r| r.kind == InheritanceKind::Implements && r.child_name == "Foo")
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert!(implements.contains(&"I1"));
        assert!(implements.contains(&"java.util.List"));
    }

    #[test]
    fn interface_extends_multiple() {
        let src = "public interface Foo extends Bar, Baz { }";
        let (_, _, _, inh, _) = run(src, "Foo.java");
        let extends: Vec<&str> = inh
            .iter()
            .filter(|r| r.kind == InheritanceKind::Extends && r.child_name == "Foo")
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert!(extends.contains(&"Bar"));
        assert!(extends.contains(&"Baz"));
    }

    #[test]
    fn same_file_canonical_with_package() {
        let src = "package com.example;\npublic class Outer { }\nclass Other { Outer field; }";
        let (types, _, _, _, _) = run(src, "Outer.java");
        let outer = types
            .iter()
            .find(|t| t.display_name == "Outer")
            .expect("Outer");
        assert_eq!(outer.canonical_name.as_deref(), Some("com.example.Outer"));
    }

    #[test]
    fn throws_emits_type_rows() {
        let src = "import java.io.IOException;\npublic class C { void f() throws IOException, RuntimeException { } }";
        let (types, _, _, _, _) = run(src, "C.java");
        let io = types
            .iter()
            .find(|t| t.display_name == "IOException")
            .expect("IOException");
        assert_eq!(io.canonical_name.as_deref(), Some("java.io.IOException"));
        let rt = types
            .iter()
            .find(|t| t.display_name == "RuntimeException")
            .expect("RuntimeException");
        assert_eq!(
            rt.canonical_name.as_deref(),
            Some("java.lang.RuntimeException")
        );
    }

    #[test]
    fn multi_catch_union() {
        let src = "import java.io.IOException;\npublic class C { void f() { try { } catch (IOException | RuntimeException e) { } } }";
        let (types, _, _, _, _) = run(src, "C.java");
        let u = types.iter().find(|t| t.kind == "union").expect("union row");
        assert_eq!(u.display_name, "IOException | RuntimeException");
        assert_eq!(
            u.canonical_name.as_deref(),
            Some("java.io.IOException | java.lang.RuntimeException")
        );
    }

    #[test]
    fn varargs_parameter() {
        let src = "public class C { public void log(String... args) { } }";
        let (types, params, _, _, _) = run(src, "C.java");
        assert_eq!(params[0].type_display_name.as_deref(), Some("String[]"));
        let arr = types
            .iter()
            .find(|t| t.display_name == "String[]")
            .expect("String[]");
        assert_eq!(arr.kind, "array");
        assert_eq!(arr.canonical_name.as_deref(), Some("java.lang.String[]"));
    }

    #[test]
    fn type_parameter_unresolved() {
        let src = "public class C<T> { public T get() { return null; } }";
        let (types, _, _, _, _) = run(src, "C.java");
        let t = types.iter().find(|t| t.display_name == "T").expect("T row");
        assert_eq!(t.kind, "named");
        assert_eq!(t.canonical_name, None);
    }
}
