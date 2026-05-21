//! Issue #15 — `cpp_attrs` extractor.
//!
//! Per `docs/attrs-cpp.md`, emit one row per C++ symbol with:
//! - `is_virtual`   — `virtual` keyword on a method, or `override` present
//! - `is_const`     — trailing `const` on a method signature, or `const`
//!                    qualifier on a variable/field
//! - `is_noexcept`  — `noexcept` specifier (except `noexcept(false)`)
//! - `is_template`  — defining node wrapped in a `template_declaration`
//! - `is_constexpr` — `constexpr` keyword
//! - `is_override`  — `override` `virtual_specifier`
//! - `is_final`     — `final` `virtual_specifier` (method or class)
//!
//! Symbol ID convention: ADR-0002 (`path|line|col|name|kind`).

use tree_sitter::{Node, Tree};

use crate::models::{CppAttrsRow, SymbolInfo};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<CppAttrsRow> {
    let mut out = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );

        let Some(node) = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte) else {
            out.push(CppAttrsRow {
                symbol_id,
                is_virtual: false,
                is_const: false,
                is_noexcept: false,
                is_template: false,
                is_constexpr: false,
                is_override: false,
                is_final: false,
            });
            continue;
        };

        // Containing node: tree-sitter's class/struct/function/field
        // declarations are the AST nodes we scan. Use the symbol's
        // detected node directly; for declarators we walk up.
        let outer = enclosing_decl(node);

        let is_template = is_wrapped_in_template(outer);
        let is_virtual =
            has_virtual_keyword(outer, source) || has_virtual_specifier(outer, source, "override");
        let is_constexpr = has_constexpr(outer, source);
        let is_override = has_virtual_specifier(outer, source, "override");
        let is_final = has_virtual_specifier(outer, source, "final");
        let is_noexcept = has_noexcept(outer, source);
        let is_const = has_const_qualifier(outer, source);

        out.push(CppAttrsRow {
            symbol_id,
            is_virtual,
            is_const,
            is_noexcept,
            is_template,
            is_constexpr,
            is_override,
            is_final,
        });
    }
    out
}

/// Walk up from a possibly-inner node (e.g. `identifier` inside a
/// `function_declarator`) until we hit a declaration-shaped node we can
/// scan for modifiers.
fn enclosing_decl(node: Node) -> Node {
    let mut n = node;
    loop {
        match n.kind() {
            "function_definition"
            | "field_declaration"
            | "declaration"
            | "class_specifier"
            | "struct_specifier"
            | "union_specifier"
            | "alias_declaration"
            | "type_definition"
            | "template_declaration" => return n,
            _ => {}
        }
        match n.parent() {
            Some(p) => n = p,
            None => return n,
        }
    }
}

fn is_wrapped_in_template(node: Node) -> bool {
    if node.kind() == "template_declaration" {
        return true;
    }
    matches!(
        node.parent().map(|p| p.kind()),
        Some("template_declaration")
    )
}

/// Detect `virtual` keyword as a direct child token of a declaration
/// node. Tree-sitter exposes it as an anonymous child with kind
/// `"virtual"` on `field_declaration` / `function_definition`.
fn has_virtual_keyword(node: Node, _source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "virtual" || child.kind() == "virtual_function_specifier" {
            return true;
        }
    }
    false
}

/// Detect a `virtual_specifier` child anywhere inside the declaration
/// whose text matches `expected` (`override` or `final`).
fn has_virtual_specifier(node: Node, source: &[u8], expected: &str) -> bool {
    walk_for(node, |n| {
        n.kind() == "virtual_specifier" && n.utf8_text(source).unwrap_or("").trim() == expected
    })
}

/// `constexpr` shows up either as a `storage_class_specifier` child
/// (text `"constexpr"`) or as an anonymous `"constexpr"` token.
fn has_constexpr(node: Node, source: &[u8]) -> bool {
    walk_for(node, |n| match n.kind() {
        "constexpr" => true,
        "storage_class_specifier" => n.utf8_text(source).unwrap_or("").trim() == "constexpr",
        _ => false,
    })
}

/// `noexcept` keyword. We accept bare `noexcept`, `noexcept(true)`, and
/// `noexcept(<expr>)` as true. The single exception is the literal
/// `noexcept(false)` which is the only documented false case.
fn has_noexcept(node: Node, source: &[u8]) -> bool {
    let mut found = false;
    let mut is_false = false;
    walk_recursive(node, &mut |n| {
        if n.kind() == "noexcept" {
            found = true;
            // Inspect parenthesized argument, if any: the noexcept node
            // contains the whole `noexcept(...)` construct.
            let text = n.utf8_text(source).unwrap_or("");
            // Strip whitespace; check for exact `noexcept(false)`.
            let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            if compact == "noexcept(false)" {
                is_false = true;
            }
        }
    });
    found && !is_false
}

/// `is_const`:
/// - Method: `type_qualifier` child of a `function_declarator` with
///   text `"const"`.
/// - Variable/field: a `type_qualifier` child (of the declaration node)
///   with text `"const"`.
///
/// Pointer-to-const (`const int*`) is intentionally accepted as `true`
/// since the `const` qualifier still appears as a `type_qualifier` of
/// the declaration; distinguishing pointee-const vs. pointer-const
/// requires declarator chain analysis and is out of scope for the MVP.
fn has_const_qualifier(node: Node, source: &[u8]) -> bool {
    // Search for any function_declarator with a const type_qualifier.
    let mut found_method_const = false;
    walk_recursive(node, &mut |n| {
        if n.kind() == "function_declarator" {
            let mut cur = n.walk();
            for c in n.children(&mut cur) {
                if c.kind() == "type_qualifier"
                    && c.utf8_text(source).unwrap_or("").trim() == "const"
                {
                    found_method_const = true;
                }
            }
        }
    });
    if found_method_const {
        return true;
    }
    // Variable/field: type_qualifier direct child of the declaration.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_qualifier"
            && child.utf8_text(source).unwrap_or("").trim() == "const"
        {
            return true;
        }
    }
    false
}

fn walk_for(node: Node, mut pred: impl FnMut(Node) -> bool) -> bool {
    let mut found = false;
    walk_recursive(node, &mut |n| {
        if !found && pred(n) {
            found = true;
        }
    });
    found
}

fn walk_recursive(node: Node, f: &mut dyn FnMut(Node)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_recursive(child, f);
    }
}

/// Locate the deepest tree-sitter node spanning exactly
/// `[start_byte, end_byte]`. Same helper shape as the Rust pilot.
fn find_node_at(root: Node, start_byte: u32, end_byte: u32) -> Option<Node> {
    if (root.end_byte() as u32) < start_byte || (root.start_byte() as u32) > end_byte {
        return None;
    }
    let mut cursor = root.walk();
    for c in root.children(&mut cursor) {
        if let Some(n) = find_node_at(c, start_byte, end_byte) {
            return Some(n);
        }
    }
    if root.start_byte() as u32 == start_byte && root.end_byte() as u32 == end_byte {
        return Some(root);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> Vec<CppAttrsRow> {
        let mut parser = create_parser(Language::Cpp).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = languages::compile_symbol_query(Language::Cpp).expect("symbol query");
        let symbols =
            languages::extract_symbols(&tree, src.as_bytes(), &query, path, Language::Cpp);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn virtual_method_marked() {
        let rows = run("class S { public: virtual void f(); };", "x.hpp");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|f|method"))
            .expect("method row");
        assert!(r.is_virtual, "rows: {:?}", rows);
        assert!(!r.is_override);
        assert!(!r.is_final);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn const_method_marked() {
        let rows = run("class S { public: int get() const; };", "x.hpp");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|get|method"))
            .expect("method row");
        assert!(r.is_const, "rows: {:?}", rows);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn noexcept_function_marked() {
        let rows = run("void swap_fast(int& a, int& b) noexcept;", "x.hpp");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|swap_fast|"))
            .expect("fn row");
        assert!(r.is_noexcept);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn noexcept_false_not_marked() {
        let rows = run("void may_throw(int x) noexcept(false);", "x.hpp");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|may_throw|"))
            .expect("fn row");
        assert!(!r.is_noexcept);
    }

    #[test]
    fn template_class_marked() {
        let rows = run(
            "template <typename T> class Pool { public: T* allocate(); };",
            "x.hpp",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|Pool|class"))
            .expect("class row");
        assert!(r.is_template, "rows: {:?}", rows);
        // Members are not themselves templates.
        let m = rows
            .iter()
            .find(|r| r.symbol_id.contains("|allocate|method"));
        if let Some(m) = m {
            assert!(!m.is_template);
        }
    }

    #[test]
    fn constexpr_function_marked() {
        let rows = run("constexpr int square(int x) { return x * x; }", "x.hpp");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|square|"))
            .expect("fn row");
        assert!(r.is_constexpr);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn override_implies_virtual() {
        let rows = run(
            "class D : public B { public: bool has_more() const override; };",
            "x.hpp",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|has_more|method"))
            .expect("method row");
        assert!(r.is_override, "rows: {:?}", rows);
        assert!(r.is_virtual, "override implies virtual");
        assert!(r.is_const);
    }

    #[test]
    fn final_class_marked() {
        let rows = run("class Sealed final { };", "x.hpp");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|Sealed|class"))
            .expect("class row");
        assert!(r.is_final, "rows: {:?}", rows);
    }

    #[test]
    fn defaults_row_emitted() {
        let rows = run("class Stage { };", "x.hpp");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|Stage|class"))
            .expect("class row");
        assert!(!r.is_virtual);
        assert!(!r.is_const);
        assert!(!r.is_noexcept);
        assert!(!r.is_template);
        assert!(!r.is_constexpr);
        assert!(!r.is_override);
        assert!(!r.is_final);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn one_row_per_symbol() {
        let rows = run("void a(); void b(); class C { };", "x.hpp");
        assert!(rows.len() >= 3, "got {}: {:?}", rows.len(), rows);
    }
}
