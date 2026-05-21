//! Issue #15 Rust pilot — `rust_attrs` extractor.
//!
//! Per contract review policy 4 we do NOT duplicate `is_async` /
//! `is_static` / `is_mutable` (already on `symbol`). The MVP columns
//! are:
//! - `is_unsafe` — `function_item` / `impl_item` / `trait_item` with
//!   the `unsafe` keyword
//! - `is_const`  — `function_item` with the `const` keyword, plus
//!   `const_item` symbols
//! - `derives`   — derive macro names from `#[derive(...)]`
//!   attributes on `struct_item` / `enum_item` / `union_item`
//!
//! Additional columns from `docs/attrs-rust.md` (is_extern / abi /
//! is_test / cfg / type_parameters / lifetime_parameters /
//! visibility_kind / is_proc_macro) are deferred to a follow-up that
//! extends the schema. We emit one row per Rust symbol regardless of
//! whether any column is non-default — that matches the contract's
//! "1:1 with symbols" rationale.
//!
//! Symbol ID convention: ADR-0002 (`path|line|col|name|kind`).

use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{RustAttrsRow, SymbolInfo, SymbolKind};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<RustAttrsRow> {
    let mut out = Vec::with_capacity(symbols.len());
    // Pre-compute (start_byte → derive list) so we can attach derives to
    // each struct/enum/union symbol without re-walking the AST per row.
    let derives_by_start = collect_derives(tree, source);
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );
        let is_unsafe = sym_has_keyword(tree, sym, source, "unsafe");
        let is_const = sym.kind == SymbolKind::Constant
            || (matches!(sym.kind, SymbolKind::Function | SymbolKind::Method)
                && sym_has_keyword(tree, sym, source, "const"));
        let derives = derives_by_start
            .get(&sym.start_byte)
            .cloned()
            .unwrap_or_default();
        out.push(RustAttrsRow {
            symbol_id,
            is_unsafe,
            is_const,
            derives,
        });
    }
    out
}

/// Find the tree-sitter node for `sym` and check whether it carries a
/// given keyword among `function_modifiers` / direct children.
fn sym_has_keyword(tree: &Tree, sym: &SymbolInfo, _source: &[u8], keyword: &str) -> bool {
    let Some(node) = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte) else {
        return false;
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == keyword {
            return true;
        }
        if child.kind() == "function_modifiers" {
            let mut inner = child.walk();
            for k in child.children(&mut inner) {
                if k.kind() == keyword {
                    return true;
                }
            }
        }
    }
    false
}

/// Walk every `attribute_item` whose path is `derive` and collect the
/// derive names indexed by the start_byte of the **next sibling**
/// definition (struct/enum/union). The next-sibling lookup mirrors how
/// `find_associated_symbol` in `queries.rs` attaches doc comments.
fn collect_derives(tree: &Tree, source: &[u8]) -> HashMap<u32, Vec<String>> {
    const Q: &str = r#"
        (attribute_item) @attr
    "#;
    let mut out: HashMap<u32, Vec<String>> = HashMap::new();
    let lang = Language::Rust.tree_sitter_language();
    let Ok(query) = Query::new(&lang, Q) else {
        return out;
    };
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        let Some(cap) = m.captures.first() else {
            continue;
        };
        let attr = cap.node;
        let Some(names) = parse_derive_attr(attr, source) else {
            continue;
        };
        // Find the next named sibling that's a derive-eligible symbol.
        let mut sib = attr.next_named_sibling();
        while let Some(s) = sib {
            match s.kind() {
                "struct_item" | "enum_item" | "union_item" => {
                    out.entry(s.start_byte() as u32).or_default().extend(names);
                    break;
                }
                "attribute_item" => {
                    // Another attribute; keep walking forward.
                    sib = s.next_named_sibling();
                    continue;
                }
                _ => break,
            }
        }
    }
    out
}

/// Parse `#[derive(A, B, C)]` → `["A", "B", "C"]`. Returns `None` if
/// the attribute isn't a `derive`.
fn parse_derive_attr(attr: Node, source: &[u8]) -> Option<Vec<String>> {
    let text = attr.utf8_text(source).ok()?.trim();
    // Cheapest check: textual match on `#[derive(`.
    let stripped = text.strip_prefix("#[")?.strip_suffix("]")?;
    let stripped = stripped.trim();
    let inner = stripped.strip_prefix("derive(")?.strip_suffix(")")?;
    let names: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() { None } else { Some(names) }
}

/// Locate the deepest tree-sitter node spanning exactly
/// `[start_byte, end_byte]`. Used to back-reference a `SymbolInfo`
/// to its AST node. Prefers a deeper match over a shallower one when
/// both have the same byte range — a single-function source file has
/// `source_file` and `function_item` covering identical bytes; we
/// want the inner definition node.
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

    fn run(src: &str, path: &str) -> Vec<RustAttrsRow> {
        let mut parser = create_parser(Language::Rust).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = languages::compile_symbol_query(Language::Rust).expect("symbol query");
        let symbols =
            languages::extract_symbols(&tree, src.as_bytes(), &query, path, Language::Rust);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn unsafe_function_marked() {
        let rows = run("unsafe fn dangerous() {}", "src/lib.rs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|dangerous|function"))
            .unwrap();
        assert!(r.is_unsafe);
        assert!(!r.is_const);
        assert!(r.derives.is_empty());
    }

    #[test]
    fn const_function_marked() {
        let rows = run("const fn forever() -> i32 { 0 }", "src/lib.rs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|forever|function"))
            .unwrap();
        assert!(r.is_const);
        assert!(!r.is_unsafe);
    }

    #[test]
    fn const_item_is_const() {
        let rows = run("const MAX: i32 = 100;", "src/lib.rs");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|MAX|constant"))
            .unwrap();
        assert!(r.is_const);
    }

    #[test]
    fn derive_attached_to_struct() {
        let rows = run(
            "#[derive(Debug, Clone)]\npub struct Foo { x: i32 }",
            "src/lib.rs",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|struct"))
            .unwrap();
        assert_eq!(r.derives, vec!["Debug".to_string(), "Clone".to_string()]);
    }

    #[test]
    fn one_row_per_symbol() {
        let rows = run("fn alpha() {}\nfn beta() {}\nstruct S;", "src/lib.rs");
        // Includes alpha, beta, S — at minimum 3.
        assert!(rows.len() >= 3, "got {}: {:?}", rows.len(), rows);
    }
}
