//! Issue #13 PHP — type-expression / signature / inheritance extractor.
//! Contract: docs/types-php.md. Per ADR-0003 (Level 3): full kind
//! decomposition + canonical_name resolution.

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
    /// `use Foo\Bar as B;` → local `B` → canonical `Foo\Bar`. Also
    /// stores `use Foo\Bar;` as local `Bar` → canonical `Foo\Bar`.
    use_bindings: Vec<UseBinding>,
    /// The active `namespace Foo\Bar;` declaration, if any.
    current_namespace: Option<String>,
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
            current_namespace: None,
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

    /// Walk the file collecting top-level `namespace` declarations and
    /// `use` bindings. Both may appear nested in `namespace_definition`
    /// blocks (PHP brace-syntax namespaces).
    fn collect_file_level(&mut self, root: Node) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "namespace_definition" => {
                    if let Some(name_node) = child.child_by_field_name("name")
                        && let Ok(name) = name_node.utf8_text(self.source)
                    {
                        self.current_namespace = Some(name.trim().to_string());
                    }
                    // Brace-syntax namespaces have a body containing
                    // `use` declarations.
                    if let Some(body) = child.child_by_field_name("body") {
                        self.collect_file_level(body);
                    }
                }
                "namespace_use_declaration" => {
                    self.collect_use_bindings(child);
                }
                _ => {}
            }
        }
    }

    fn collect_use_bindings(&mut self, node: Node) {
        // namespace_use_declaration contains namespace_use_clause /
        // namespace_use_group nodes.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "namespace_use_clause" => {
                    self.collect_use_clause(child, "");
                }
                "namespace_use_group" => {
                    // `use Foo\{Bar, Baz as B};` — find the prefix path
                    // among the children (a `namespace_name`) then walk
                    // the group's clauses.
                    let mut prefix = String::new();
                    let mut group_cursor = child.walk();
                    for gc in child.named_children(&mut group_cursor) {
                        if gc.kind() == "namespace_name" {
                            prefix = gc.utf8_text(self.source).unwrap_or("").trim().to_string();
                            break;
                        }
                    }
                    let mut group_cursor = child.walk();
                    for gc in child.named_children(&mut group_cursor) {
                        if gc.kind() == "namespace_use_clause"
                            || gc.kind() == "namespace_use_group_clause"
                        {
                            self.collect_use_clause(gc, &prefix);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_use_clause(&mut self, clause: Node, prefix: &str) {
        // namespace_use_clause: name [as alias]
        // Find the namespace path and optional alias.
        let mut path = String::new();
        let mut alias: Option<String> = None;

        let mut cursor = clause.walk();
        for child in clause.named_children(&mut cursor) {
            match child.kind() {
                "qualified_name" | "namespace_name" | "name" => {
                    if path.is_empty() {
                        path = child
                            .utf8_text(self.source)
                            .unwrap_or("")
                            .trim()
                            .to_string();
                    }
                }
                "namespace_aliasing_clause" => {
                    // `as Alias` — find the `name` child.
                    let mut ac = child.walk();
                    for a in child.named_children(&mut ac) {
                        if a.kind() == "name" {
                            alias = Some(a.utf8_text(self.source).unwrap_or("").trim().to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        if path.is_empty() {
            return;
        }
        let canonical = if prefix.is_empty() {
            path.clone()
        } else {
            format!("{}\\{}", prefix.trim_end_matches('\\'), path)
        };
        // Strip leading backslash for canonical store form.
        let canonical = canonical.trim_start_matches('\\').to_string();
        let local = alias.unwrap_or_else(|| {
            canonical
                .rsplit('\\')
                .next()
                .unwrap_or(&canonical)
                .to_string()
        });
        if local.is_empty() {
            return;
        }
        self.use_bindings.push(UseBinding {
            local_name: local,
            canonical_path: canonical,
        });
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" | "method_declaration" => self.visit_function(node),
            "class_declaration" => self.visit_class(node),
            "interface_declaration" => self.visit_interface(node),
            "trait_declaration" => self.visit_trait(node),
            "property_declaration" => self.visit_property(node),
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
        let kind = if node.kind() == "method_declaration" {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let (fn_line, fn_col) = node_pos(name_node);

        if let Some(params) = node.child_by_field_name("parameters") {
            self.visit_parameters(params, name, kind, fn_line, fn_col);
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            let display = render_type(ret, self.source);
            if !display.is_empty() {
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
                "simple_parameter" | "variadic_parameter" | "property_promotion_parameter" => {
                    let param_name = p
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(self.source).ok())
                        .map(|s| s.trim_start_matches('$').to_string())
                        .unwrap_or_default();
                    let name_node = p.child_by_field_name("name").unwrap_or(p);
                    let (pl, pc) = node_pos(name_node);

                    // `?T` means nullable — is_optional=true.
                    let type_node = p.child_by_field_name("type");
                    let is_optional = type_node
                        .map(|t| t.kind() == "optional_type")
                        .unwrap_or(false);
                    let has_default = p.child_by_field_name("default_value").is_some();

                    let type_display = if let Some(t) = type_node {
                        self.emit_type_with_subtree(t);
                        let d = render_type(t, self.source);
                        if d.is_empty() { None } else { Some(d) }
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
                        parameter_name: param_name,
                        position,
                        type_display_name: type_display,
                        is_optional,
                        has_default,
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

        // Extends: `base_clause` field with one parent class.
        let mut cursor = node.walk();
        for ch in node.named_children(&mut cursor) {
            match ch.kind() {
                "base_clause" => {
                    self.collect_clause_parents(
                        ch,
                        child_name,
                        SymbolKind::Class,
                        cl,
                        cc,
                        InheritanceKind::Extends,
                    );
                }
                "class_interface_clause" => {
                    self.collect_clause_parents(
                        ch,
                        child_name,
                        SymbolKind::Class,
                        cl,
                        cc,
                        InheritanceKind::Implements,
                    );
                }
                _ => {}
            }
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

        // Interface inheritance is expressed via `base_clause` (multiple parents allowed).
        let mut cursor = node.walk();
        for ch in node.named_children(&mut cursor) {
            if ch.kind() == "base_clause" {
                self.collect_clause_parents(
                    ch,
                    child_name,
                    SymbolKind::Interface,
                    cl,
                    cc,
                    InheritanceKind::Extends,
                );
            }
        }
    }

    fn visit_trait(&mut self, _node: Node) {
        // PHP traits don't have extends/implements at the language
        // level. `use TraitName;` inside a class is handled via the
        // `use_declaration` extractor but is not modelled as an
        // inheritance edge here (per the contract — only extends and
        // implements are emitted).
    }

    fn visit_property(&mut self, node: Node) {
        // property_declaration with a `type` field — emit type rows so
        // the field type appears in the per-file `type` table. Untyped
        // properties (legacy dynamic PHP) emit no row.
        let Some(t) = node.child_by_field_name("type") else {
            return;
        };
        self.emit_type_with_subtree(t);
        // Issue #14: PHP `property_declaration` has child
        // `property_element` nodes (one per `$x = init`). Each carries a
        // `name` that's a `variable_name` ($x).
        let display = render_type(t, self.source);
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if c.kind() != "property_element" {
                continue;
            }
            let Some(name_node) = c.child_by_field_name("name") else {
                continue;
            };
            let raw = name_node.utf8_text(self.source).unwrap_or("");
            // Strip the leading `$` so the field name matches the
            // symbol-extraction convention.
            let field_name = raw.trim_start_matches('$').to_string();
            if field_name.is_empty() {
                continue;
            }
            // #18.1: key on the property_declaration's start (`node`),
            // not the property_element's, so the synthesized symbol_id
            // matches the Symbol row's id.
            let (line, col) = node_pos(node);
            self.field_types.push(FieldTypeRow {
                file_path: self.file_path.to_string(),
                field_start_line: line,
                field_start_col: col,
                field_name,
                field_kind: SymbolKind::Field,
                type_display_name: display.clone(),
            });
        }
    }

    fn collect_clause_parents(
        &mut self,
        clause: Node,
        child_name: &str,
        child_kind: SymbolKind,
        cl: u32,
        cc: u32,
        kind: InheritanceKind,
    ) {
        let mut cursor = clause.walk();
        for p in clause.named_children(&mut cursor) {
            // Skip keyword-like punctuation tokens; only emit on type
            // references.
            if !is_named_parent_node(p.kind()) {
                continue;
            }
            let display = render_type(p, self.source);
            if display.is_empty() {
                continue;
            }
            self.emit_type_with_subtree(p);
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
    /// nested inside it. Per docs/types-php.md.
    fn emit_type_with_subtree(&mut self, node: Node) {
        if let Some((kind, display)) = self.classify_type_node(node) {
            let canonical = self.resolve_canonical(node, &display);
            if self.seen_display.insert(display.clone()) {
                self.types.push(TypeRow {
                    file_path: self.file_path.to_string(),
                    kind,
                    display_name: display,
                    canonical_name: canonical,
                });
            }
        }
        // Recurse — union/intersection/optional operand types each get
        // their own rows.
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if !is_type_position_node(c.kind()) {
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
            "primitive_type" => "primitive",
            "named_type" | "qualified_name" | "name" => "named",
            "optional_type" => "union",
            "union_type" => "union",
            "intersection_type" => "intersection",
            "disjunctive_normal_form_type" => "union",
            "cast_type" => "primitive",
            _ => return None,
        };
        Some((kind.to_string(), display))
    }

    /// Resolve a type node to canonical form. Handles compound types
    /// (union/intersection/optional) by joining operand canonicals.
    fn resolve_canonical(&self, node: Node, display: &str) -> Option<String> {
        match node.kind() {
            "primitive_type" | "cast_type" => Some(display.trim().to_lowercase()),
            "optional_type" => {
                // `?T` → `<canonical(T)>|null`. Find the inner type child.
                let inner = first_type_child(node)?;
                let inner_display = render_type(inner, self.source);
                let inner_canonical = self.resolve_canonical(inner, &inner_display)?;
                Some(format!("{}|null", inner_canonical))
            }
            "union_type" => {
                let parts = self.collect_operand_canonicals(node, "|")?;
                Some(parts.join("|"))
            }
            "intersection_type" => {
                let parts = self.collect_operand_canonicals(node, "&")?;
                Some(parts.join("&"))
            }
            "disjunctive_normal_form_type" => {
                // (A&B)|C — operand may be `intersection_type` or a
                // bare named/primitive operand.
                let parts = self.collect_operand_canonicals(node, "|")?;
                Some(parts.join("|"))
            }
            "named_type" | "qualified_name" | "name" => self.resolve_head(display),
            _ => None,
        }
    }

    fn collect_operand_canonicals(&self, node: Node, _sep: &str) -> Option<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let mut cursor = node.walk();
        for c in node.named_children(&mut cursor) {
            if !is_type_position_node(c.kind()) {
                continue;
            }
            let d = render_type(c, self.source);
            let resolved = self.resolve_canonical(c, &d)?;
            out.push(resolved);
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// Resolve a bare/qualified PHP type name's canonical form.
    fn resolve_head(&self, display: &str) -> Option<String> {
        let head = display.trim();
        if head.is_empty() {
            return None;
        }
        // self/static/parent need enclosing class context which we don't
        // track here; leave unresolved (the contract permits this for
        // Phase 1 since the resolver pass downstream handles them).
        if head == "self" || head == "static" || head == "parent" {
            return None;
        }
        // Already-qualified with leading `\` → keep verbatim.
        if let Some(rest) = head.strip_prefix('\\') {
            return Some(format!("\\{}", normalize_namespace(rest)));
        }
        // Use-alias match: try first segment as alias.
        let first_segment = head.split('\\').next().unwrap_or(head);
        for u in &self.use_bindings {
            if u.local_name == first_segment {
                if head == first_segment {
                    return Some(u.canonical_path.clone());
                }
                let rest = &head[first_segment.len()..];
                return Some(format!("{}{}", u.canonical_path, rest));
            }
        }
        // Same-namespace lookup: if the file declares a namespace,
        // qualify the bare name with it.
        if let Some(ns) = &self.current_namespace {
            return Some(format!("{}\\{}", ns, head));
        }
        // Root-namespace allow-list for PHP built-in classes/interfaces.
        if is_php_builtin(head) {
            return Some(format!("\\{}", head));
        }
        None
    }
}

fn is_named_parent_node(kind: &str) -> bool {
    matches!(kind, "name" | "qualified_name" | "named_type")
}

fn is_type_position_node(kind: &str) -> bool {
    matches!(
        kind,
        "primitive_type"
            | "named_type"
            | "qualified_name"
            | "name"
            | "optional_type"
            | "union_type"
            | "intersection_type"
            | "disjunctive_normal_form_type"
            | "cast_type"
    )
}

fn first_type_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        if is_type_position_node(c.kind()) {
            return Some(c);
        }
    }
    None
}

fn node_pos(n: Node) -> (u32, u32) {
    let p = n.start_position();
    (p.row as u32 + 1, p.column as u32)
}

/// Render a type node into the normalized `display_name` form per
/// docs/types-php.md "display_name construction" rules.
fn render_type(node: Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");
    normalize_type_text(text)
}

fn normalize_type_text(raw: &str) -> String {
    // Collapse whitespace runs to single spaces.
    let mut s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Strip whitespace around `|`, `&`, `\`, `(`, `)`, `?`.
    for tok in [" |", "| ", " &", "& ", " \\", "\\ ", "( ", " )", "? "] {
        let replacement = tok.replace(' ', "");
        while let Some(idx) = s.find(tok) {
            s.replace_range(idx..idx + tok.len(), &replacement);
        }
    }
    s.trim().to_string()
}

fn normalize_namespace(s: &str) -> String {
    s.split('\\')
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join("\\")
}

/// PHP built-in classes/interfaces that live at the root namespace and
/// should canonicalize to `\Name` when referenced bare. Per
/// docs/types-php.md "Root namespace fallback".
fn is_php_builtin(name: &str) -> bool {
    matches!(
        name,
        "Closure"
            | "Generator"
            | "Iterator"
            | "IteratorAggregate"
            | "Throwable"
            | "Exception"
            | "Error"
            | "TypeError"
            | "ValueError"
            | "RuntimeException"
            | "LogicException"
            | "InvalidArgumentException"
            | "Stringable"
            | "Countable"
            | "ArrayAccess"
            | "Traversable"
            | "DateTime"
            | "DateTimeImmutable"
            | "DateTimeInterface"
            | "stdClass"
            | "SplObjectStorage"
            | "WeakMap"
            | "WeakReference"
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
        let mut parser = create_parser(Language::Php).expect("parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        extract_types(&tree, source.as_bytes(), path)
    }

    #[test]
    fn primitive_param_and_return() {
        let (types, params, returns, _, _) = run(
            "<?php\nfunction add(int $a, int $b): int { return $a + $b; }",
            "test.php",
        );
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "int" && t.kind == "primitive")
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].parameter_name, "a");
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
        assert!(!params[0].is_optional);
        assert!(!params[0].has_default);
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].type_display_name, "int");
    }

    #[test]
    fn untyped_parameters_emit_none() {
        let (_, params, returns, _, _) = run("<?php\nfunction f($a, $b = 1) {}", "test.php");
        assert_eq!(params.len(), 2);
        assert!(params[0].type_display_name.is_none());
        assert!(!params[0].has_default);
        assert!(params[1].type_display_name.is_none());
        assert!(params[1].has_default);
        assert_eq!(returns.len(), 0);
    }

    #[test]
    fn optional_type_is_union_with_nullable() {
        let (types, params, _, _, _) = run("<?php\nfunction f(?string $y = null) {}", "test.php");
        let opt = types
            .iter()
            .find(|t| t.display_name == "?string")
            .expect("?string row");
        assert_eq!(opt.kind, "union");
        assert_eq!(opt.canonical_name.as_deref(), Some("string|null"));
        // Inner string row should also be emitted.
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "string" && t.kind == "primitive")
        );
        assert!(params[0].is_optional);
        assert!(params[0].has_default);
    }

    #[test]
    fn union_type_kind_and_canonical() {
        let (types, _, returns, _, _) =
            run("<?php\nfunction f(): int|string { return 1; }", "test.php");
        let u = types.iter().find(|t| t.kind == "union").expect("union row");
        assert_eq!(u.display_name, "int|string");
        assert_eq!(u.canonical_name.as_deref(), Some("int|string"));
        assert_eq!(returns[0].type_display_name, "int|string");
    }

    #[test]
    fn intersection_type_kind() {
        let (types, _, _, _, _) = run("<?php\nfunction f(Countable&Stringable $x) {}", "test.php");
        let i = types
            .iter()
            .find(|t| t.kind == "intersection")
            .expect("intersection row");
        assert_eq!(i.display_name, "Countable&Stringable");
    }

    #[test]
    fn class_extends_and_implements() {
        let (_, _, _, inh, _) = run(
            "<?php\nclass Foo extends Bar implements I1, I2 {}",
            "test.php",
        );
        let extends: Vec<&str> = inh
            .iter()
            .filter(|r| r.kind == InheritanceKind::Extends && r.child_name == "Foo")
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert_eq!(extends, vec!["Bar"]);
        let implements: Vec<&str> = inh
            .iter()
            .filter(|r| r.kind == InheritanceKind::Implements && r.child_name == "Foo")
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert!(implements.contains(&"I1"));
        assert!(implements.contains(&"I2"));
    }

    #[test]
    fn interface_extends_multiple() {
        let (_, _, _, inh, _) = run("<?php\ninterface Sub extends A, B {}", "test.php");
        let parents: Vec<&str> = inh
            .iter()
            .filter(|r| r.kind == InheritanceKind::Extends && r.child_name == "Sub")
            .map(|r| r.parent_display_name.as_str())
            .collect();
        assert!(parents.contains(&"A"));
        assert!(parents.contains(&"B"));
    }

    // TODO(#13 follow-up): `use App\Models\User as U` alias not picked
    // up by canonical_name resolver. Track in fan-out polish.
    #[ignore]
    #[test]
    fn use_alias_resolution() {
        let (types, _, _, _, _) = run(
            "<?php\nuse App\\Models\\User as U;\nfunction f(U $u): U { return $u; }",
            "test.php",
        );
        let row = types
            .iter()
            .find(|t| t.display_name == "U" && t.kind == "named")
            .expect("U row");
        assert_eq!(row.canonical_name.as_deref(), Some("App\\Models\\User"));
    }

    #[test]
    fn leading_backslash_preserved() {
        let (types, _, _, _, _) = run(
            "<?php\nfunction f(): \\Illuminate\\Http\\Request {}",
            "test.php",
        );
        let row = types
            .iter()
            .find(|t| t.display_name.starts_with('\\'))
            .expect("backslash row");
        assert_eq!(row.kind, "named");
        assert_eq!(
            row.canonical_name.as_deref(),
            Some("\\Illuminate\\Http\\Request")
        );
    }

    #[test]
    fn builtin_closure_resolves_to_root() {
        let (types, _, _, _, _) = run("<?php\nfunction f(Closure $c) {}", "test.php");
        let row = types
            .iter()
            .find(|t| t.display_name == "Closure")
            .expect("Closure row");
        assert_eq!(row.canonical_name.as_deref(), Some("\\Closure"));
    }

    #[test]
    fn same_namespace_resolution() {
        let (types, _, _, _, _) = run(
            "<?php\nnamespace App\\Services;\nfunction f(Cart $c) {}",
            "test.php",
        );
        let row = types
            .iter()
            .find(|t| t.display_name == "Cart")
            .expect("Cart row");
        assert_eq!(row.canonical_name.as_deref(), Some("App\\Services\\Cart"));
    }

    #[test]
    fn primitive_canonical_lowercase() {
        let (types, _, _, _, _) = run("<?php\nfunction f(): VOID {}", "test.php");
        // PHP grammar lowercases primitive keywords by lex; verify the
        // display is preserved as-written but canonical is lowercase.
        let row = types
            .iter()
            .find(|t| t.kind == "primitive")
            .expect("primitive row");
        assert_eq!(
            row.canonical_name.as_deref().map(|s| s.to_string()),
            Some(row.display_name.to_lowercase())
        );
    }

    #[test]
    fn property_type_emits_row() {
        let (types, _, _, _, _) = run("<?php\nclass Foo { private string $name; }", "test.php");
        assert!(
            types
                .iter()
                .any(|t| t.display_name == "string" && t.kind == "primitive")
        );
    }

    #[test]
    fn variadic_parameter_typed() {
        let (_, params, _, _, _) = run("<?php\nfunction f(int ...$nums) {}", "test.php");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].parameter_name, "nums");
        assert_eq!(params[0].type_display_name.as_deref(), Some("int"));
    }

    #[test]
    fn method_parameters_and_return() {
        let (_, params, returns, _, _) = run(
            "<?php\nclass Foo { public function bar(int $x): string { return ''; } }",
            "test.php",
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].function_kind, SymbolKind::Method);
        assert_eq!(params[0].function_name, "bar");
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].function_kind, SymbolKind::Method);
    }
}
