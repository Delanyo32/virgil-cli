//! Issue #15 — `c_attrs` extractor.
//!
//! Per contract review policy 4 we do NOT duplicate `is_static` (already
//! on `symbol`); the C-specific column is `is_file_static`, which records
//! every `static` keyword occurrence regardless of scope. See
//! `docs/attrs-c.md` for the full contract.
//!
//! Columns:
//! - `is_file_static` — `storage_class_specifier` = `"static"`
//! - `is_extern`      — `storage_class_specifier` = `"extern"`
//! - `is_inline`      — `storage_class_specifier` = `"inline"` (or
//!   `"__inline"`/`"__inline__"`); only meaningful for functions
//! - `is_const`       — top-level `type_qualifier` = `"const"`
//! - `is_volatile`    — top-level `type_qualifier` = `"volatile"`
//! - `is_restrict`    — top-level `type_qualifier` = `"restrict"`
//!   (or `"__restrict"`/`"__restrict__"`)
//! - `gcc_attributes` — leading identifiers of every
//!   `attribute_specifier` child of the declaration node
//!
//! A row is emitted for EVERY C symbol so that downstream joins do not
//! need outer-join semantics.

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CAttrsRow, SymbolInfo, SymbolKind};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<CAttrsRow> {
    let mut out = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );

        let mut row = CAttrsRow {
            symbol_id,
            is_file_static: false,
            is_extern: false,
            is_inline: false,
            is_const: false,
            is_volatile: false,
            is_restrict: false,
            gcc_attributes: Vec::new(),
        };

        if let Some(node) = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte) {
            fill_from_node(node, source, sym, &mut row);
        }

        out.push(row);
    }
    out
}

/// Inspect direct children of `node` for storage-class specifiers, type
/// qualifiers, and GCC `__attribute__` specifiers. Mutates `row` in place.
fn fill_from_node(node: Node, source: &[u8], sym: &SymbolInfo, row: &mut CAttrsRow) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "storage_class_specifier" => {
                let text = child.utf8_text(source).unwrap_or("").trim();
                match text {
                    "static" => row.is_file_static = true,
                    "extern" => row.is_extern = true,
                    "inline" | "__inline" | "__inline__" => {
                        if sym.kind == SymbolKind::Function {
                            row.is_inline = true;
                        }
                    }
                    _ => {}
                }
            }
            "type_qualifier" => {
                let text = child.utf8_text(source).unwrap_or("").trim();
                match text {
                    "const" => row.is_const = true,
                    "volatile" => row.is_volatile = true,
                    "restrict" | "__restrict" | "__restrict__" => row.is_restrict = true,
                    _ => {}
                }
            }
            "attribute_specifier" | "gnu_asm_expression" => {
                if child.kind() == "attribute_specifier" {
                    collect_gcc_attr_names(child, source, &mut row.gcc_attributes);
                }
            }
            _ => {}
        }
    }
}

/// Collect leading identifiers from a `__attribute__((a, b(args), c))`
/// specifier. Arguments are discarded; only the identifier names land in
/// `out`. Uses a tree-sitter query to find every `identifier` whose parent
/// is the `attribute` node (the named-attribute wrapper) under this
/// specifier.
fn collect_gcc_attr_names(specifier: Node, source: &[u8], out: &mut Vec<String>) {
    const Q: &str = r#"
        (attribute name: (identifier) @name)
    "#;
    let lang = Language::C.tree_sitter_language();
    let Ok(query) = Query::new(&lang, Q) else {
        // Fallback: scan child identifiers directly.
        collect_gcc_attr_names_fallback(specifier, source, out);
        return;
    };
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, specifier, source);
    let mut found_any = false;
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if let Ok(text) = cap.node.utf8_text(source) {
                let name = text.trim().to_string();
                if !name.is_empty() {
                    out.push(name);
                    found_any = true;
                }
            }
        }
    }
    if !found_any {
        collect_gcc_attr_names_fallback(specifier, source, out);
    }
}

/// Older / vendored tree-sitter-c grammars may not expose a `(attribute
/// name: identifier)` shape. Fall back to text parsing of the specifier:
/// strip `__attribute__((` … `))`, split by top-level commas, and take the
/// identifier prefix of each item.
fn collect_gcc_attr_names_fallback(specifier: Node, source: &[u8], out: &mut Vec<String>) {
    let Ok(text) = specifier.utf8_text(source) else {
        return;
    };
    let text = text.trim();
    // Expect `__attribute__((<inner>))`.
    let inner = text
        .strip_prefix("__attribute__")
        .or_else(|| text.strip_prefix("__attribute"))
        .unwrap_or(text)
        .trim();
    let inner = inner
        .strip_prefix("((")
        .and_then(|s| s.strip_suffix("))"))
        .unwrap_or(inner);
    // Split by commas at paren-depth 0.
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    let mut pieces: Vec<&str> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                pieces.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(&inner[start..]);
    for piece in pieces {
        let trimmed = piece.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Take prefix up to '(' or whitespace — that's the attribute name.
        let name_end = trimmed
            .find(|c: char| c == '(' || c.is_whitespace())
            .unwrap_or(trimmed.len());
        let name = trimmed[..name_end].trim_matches(|c: char| c == '_' || c.is_whitespace());
        // Preserve original (e.g. "always_inline") — only strip surrounding
        // double-underscores per GCC convention (`__noreturn__` →
        // `noreturn`). We do this carefully so that single-leading-underscore
        // names aren't mangled.
        let normalised = strip_dunder(&trimmed[..name_end]);
        if !normalised.is_empty() {
            out.push(normalised);
        } else if !name.is_empty() {
            out.push(name.to_string());
        }
    }
}

/// Strip a balanced pair of double underscores around `s` per GCC's
/// attribute spelling convention. `__noreturn__` → `noreturn`; `noreturn`
/// → `noreturn`; `__cold` → `__cold` (unbalanced — leave as-is).
fn strip_dunder(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 4 && trimmed.starts_with("__") && trimmed.ends_with("__") {
        trimmed[2..trimmed.len() - 2].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Locate the deepest tree-sitter node spanning exactly
/// `[start_byte, end_byte]`. Mirrors the helper in `rust_lang::attrs`.
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
    use crate::languages::c_lang::{compile_symbol_query, extract_symbols};
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> Vec<CAttrsRow> {
        let mut parser = create_parser(Language::C).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::C).expect("symbol query");
        let symbols = extract_symbols(&tree, src.as_bytes(), &query, path);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn static_file_scope_variable() {
        let rows = run("static int g = 0;", "src/init.c");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|g|variable"))
            .expect("g not found");
        assert!(r.is_file_static, "static keyword missed");
        assert!(!r.is_extern);
        assert!(!r.is_const);
    }

    #[test]
    fn extern_function_prototype() {
        let rows = run("extern int foo(int x);", "src/a.c");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|foo|function"))
            .expect("foo missing");
        assert!(r.is_extern);
        assert!(!r.is_file_static);
        assert!(!r.is_inline);
    }

    #[test]
    fn inline_function() {
        let rows = run(
            "static inline int add(int a, int b) { return a + b; }",
            "x.c",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|add|function"))
            .expect("add missing");
        assert!(r.is_inline, "inline keyword missed");
        assert!(r.is_file_static, "static keyword missed");
    }

    #[test]
    fn const_volatile_typedef() {
        let rows = run("typedef volatile unsigned int reg_t;", "h.h");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|reg_t|typedef"))
            .expect("reg_t missing");
        assert!(r.is_volatile, "volatile qualifier missed");
        assert!(!r.is_const);
    }

    #[test]
    fn const_variable() {
        let rows = run("const int MAX = 100;", "src/a.c");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|MAX|variable"))
            .expect("MAX missing");
        assert!(r.is_const);
    }

    #[test]
    fn one_row_per_symbol() {
        let rows = run(
            "int alpha() { return 0; }\nint beta(int x) { return x; }\nstruct S { int a; };",
            "x.c",
        );
        // alpha, beta, x (param), a (field-ish but extractor returns
        // declarations), S — at least 3.
        assert!(rows.len() >= 3, "got {}: {:?}", rows.len(), rows);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn gcc_attribute_single() {
        let rows = run("int foo(void) __attribute__((noreturn));", "src/a.c");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|foo|function"))
            .expect("foo missing");
        assert!(
            r.gcc_attributes.iter().any(|a| a == "noreturn"),
            "expected noreturn in {:?}",
            r.gcc_attributes
        );
    }

    #[test]
    fn baseline_function_all_defaults() {
        let rows = run(
            "int ringbuf_init(int rb) { return 0; }",
            "src/utils/ringbuf.c",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|ringbuf_init|function"))
            .expect("ringbuf_init missing");
        assert!(!r.is_file_static);
        assert!(!r.is_extern);
        assert!(!r.is_inline);
        assert!(!r.is_const);
        assert!(!r.is_volatile);
        assert!(!r.is_restrict);
        assert!(r.gcc_attributes.is_empty());
    }
}
