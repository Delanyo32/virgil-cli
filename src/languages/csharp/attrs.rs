//! Issue #15 — `csharp_attrs` extractor.
//!
//! MVP columns (matching `CsharpAttrsRow` in `src/models.rs`):
//! - `attributes` — names of `[X]` annotations on the declaration (name
//!   only, args dropped). Target specifiers like `[assembly: ...]` /
//!   `[return: ...]` are skipped.
//! - `is_partial` — `partial` modifier on the declaration.
//! - `is_sealed` — `sealed` modifier on the declaration.
//!
//! Additional columns from `docs/attrs-csharp.md` (is_virtual,
//! is_override, is_extern, is_unsafe) are deferred to a follow-up that
//! extends the schema. We emit one row per C# symbol regardless of
//! whether any column is non-default — matches the Rust pilot's
//! "1:1 with symbols" pattern.
//!
//! Symbol ID convention: ADR-0002 (`path|line|col|name|kind`).

use tree_sitter::{Node, Tree};

use crate::models::{CsharpAttrsRow, SymbolInfo};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<CsharpAttrsRow> {
    let mut out = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );
        let (attributes, is_partial, is_sealed) =
            match find_node_at(tree.root_node(), sym.start_byte, sym.end_byte) {
                Some(node) => {
                    let attributes = collect_attribute_names(node, source);
                    let is_partial = has_modifier(node, source, "partial");
                    let is_sealed = has_modifier(node, source, "sealed");
                    (attributes, is_partial, is_sealed)
                }
                None => (Vec::new(), false, false),
            };
        out.push(CsharpAttrsRow {
            symbol_id,
            attributes,
            is_partial,
            is_sealed,
        });
    }
    out
}

/// True if any direct `modifier` child of `node` matches `keyword`.
fn has_modifier(node: Node, source: &[u8], keyword: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" && child.utf8_text(source).unwrap_or("").trim() == keyword {
            return true;
        }
    }
    false
}

/// Walk direct `attribute_list` children of `node` and pull out each
/// inner `attribute`'s name, in source order. Target-specified
/// attribute lists (`[assembly: ...]`, `[return: ...]`) are skipped —
/// they do not attach to a declaration symbol.
fn collect_attribute_names(node: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "attribute_list" {
            continue;
        }
        if has_attribute_target(child) {
            continue;
        }
        let mut inner = child.walk();
        for grand in child.children(&mut inner) {
            if grand.kind() != "attribute" {
                continue;
            }
            if let Some(name) = attribute_name(grand, source) {
                names.push(name);
            }
        }
    }
    names
}

/// True if `attribute_list` carries a target specifier like
/// `[assembly: ...]` or `[return: ...]`. The tree-sitter C# grammar
/// exposes this as an `attribute_target_specifier` child.
fn has_attribute_target(attribute_list: Node) -> bool {
    let mut cursor = attribute_list.walk();
    for child in attribute_list.children(&mut cursor) {
        if child.kind() == "attribute_target_specifier" {
            return true;
        }
    }
    false
}

/// Return the textual name of an `attribute` node, stripping generic
/// type args (`GenericAttr<T>` → `GenericAttr`). Preserves qualified
/// names verbatim (e.g. `System.Required`) and the `Attribute` suffix
/// when written in source.
fn attribute_name(attribute: Node, source: &[u8]) -> Option<String> {
    let name_node = attribute
        .child_by_field_name("name")
        .or_else(|| first_name_like_child(attribute))?;
    let raw = name_node.utf8_text(source).ok()?.trim();
    // Generic attributes (`[GenericAttr<T>]`): keep up to the `<`.
    let trimmed = match raw.find('<') {
        Some(idx) => &raw[..idx],
        None => raw,
    };
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Fallback when `attribute` has no `name` field — find the first
/// child node that looks like an attribute name.
fn first_name_like_child(attribute: Node) -> Option<Node> {
    let mut cursor = attribute.walk();
    for child in attribute.children(&mut cursor) {
        match child.kind() {
            "identifier" | "qualified_name" | "generic_name" | "identifier_name" => {
                return Some(child);
            }
            _ => {}
        }
    }
    None
}

/// Locate the deepest tree-sitter node spanning exactly
/// `[start_byte, end_byte]`. Mirrors the helper in the Rust pilot.
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
    use crate::languages::csharp::queries::{compile_symbol_query, extract_symbols};
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> Vec<CsharpAttrsRow> {
        let mut parser = create_parser(Language::CSharp).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::CSharp).expect("symbol query");
        let symbols = extract_symbols(&tree, src.as_bytes(), &query, path);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn class_with_multiple_attributes() {
        let src = "[Authorize]\n[ApiController]\n[Route(\"api/[controller]\")]\npublic class C { }";
        let rows = run(src, "C.cs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|C|class"))
            .expect("row for C");
        assert_eq!(
            r.attributes,
            vec![
                "Authorize".to_string(),
                "ApiController".to_string(),
                "Route".to_string(),
            ]
        );
        assert!(!r.is_partial);
        assert!(!r.is_sealed);
    }

    #[test]
    fn partial_class_marked() {
        let rows = run("public partial class Foo { }", "Foo.cs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|class"))
            .unwrap();
        assert!(r.is_partial);
        assert!(!r.is_sealed);
    }

    #[test]
    fn sealed_class_marked() {
        let rows = run("public sealed class Bar { }", "Bar.cs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Bar|class"))
            .unwrap();
        assert!(r.is_sealed);
        assert!(!r.is_partial);
    }

    #[test]
    fn method_with_attribute_and_no_modifiers() {
        let src = "public class C { [HttpGet] public void GetAll() { } }";
        let rows = run(src, "C.cs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|GetAll|method"))
            .unwrap();
        assert_eq!(r.attributes, vec!["HttpGet".to_string()]);
        assert!(!r.is_partial);
        assert!(!r.is_sealed);
    }

    #[test]
    fn no_attributes_no_modifiers_yields_defaults() {
        let rows = run("public class Plain { }", "Plain.cs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Plain|class"))
            .unwrap();
        assert!(r.attributes.is_empty());
        assert!(!r.is_partial);
        assert!(!r.is_sealed);
    }

    #[test]
    fn argument_is_dropped_keep_only_name() {
        let src = "public class M { [MaxLength(200)] public string Name { get; set; } }";
        let rows = run(src, "M.cs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Name|field"))
            .unwrap();
        assert_eq!(r.attributes, vec!["MaxLength".to_string()]);
    }

    #[test]
    fn one_row_per_symbol() {
        let rows = run(
            "public class A { } public class B { } public class C { }",
            "x.cs",
        );
        // At least one row per top-level class.
        assert!(
            rows.iter()
                .filter(|r| r.symbol_id.ends_with("|class"))
                .count()
                >= 3,
            "got {:?}",
            rows
        );
    }
}
