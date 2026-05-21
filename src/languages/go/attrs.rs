//! Issue #15 — `go_attrs` extractor.
//!
//! Per `docs/attrs-go.md`, MVP columns are:
//! - `is_exported`  — first rune of the symbol name is uppercase
//! - `has_receiver` — definition node is `method_declaration`
//! - `build_tags`   — file preamble `//go:build` / `// +build` lines,
//!   broadcast to every symbol declared in the file
//!
//! Symbol ID convention: ADR-0002 (`path|line|col|name|kind`).
//! One row is emitted per Go symbol, even when all columns are default.

use tree_sitter::{Node, Tree};

use crate::models::{GoAttrsRow, SymbolInfo, SymbolKind};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<GoAttrsRow> {
    let build_tags = collect_build_tags(tree, source);
    let mut out = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );
        let is_exported = sym.name.chars().next().is_some_and(|c| c.is_uppercase());
        let has_receiver = matches!(sym.kind, SymbolKind::Function | SymbolKind::Method)
            && sym_is_method_decl(tree, sym);
        out.push(GoAttrsRow {
            symbol_id,
            is_exported,
            has_receiver,
            build_tags: build_tags.clone(),
        });
    }
    out
}

/// True iff the tree-sitter node spanning `sym` is a `method_declaration`.
fn sym_is_method_decl(tree: &Tree, sym: &SymbolInfo) -> bool {
    let Some(node) = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte) else {
        return false;
    };
    node.kind() == "method_declaration"
}

/// Walk the file preamble — the contiguous run of top-level comments
/// before the `package` clause — and collect `go:build` / `+build`
/// expressions verbatim, in source order.
fn collect_build_tags(tree: &Tree, source: &[u8]) -> Vec<String> {
    let root = tree.root_node();
    let mut tags: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "comment" => {
                let text = child.utf8_text(source).unwrap_or("");
                if let Some(expr) = parse_build_tag(text) {
                    tags.push(expr);
                }
            }
            // Stop scanning once we leave the preamble. The `package`
            // clause is always the first non-comment top-level node.
            "package_clause" => break,
            // Any other top-level construct also terminates the preamble
            // (defensive — Go source must start with `package`).
            _ => break,
        }
    }
    tags
}

/// Parse a single comment's raw text. Returns the build-tag expression
/// (text after the `go:build ` / `+build ` prefix, trimmed) or `None`
/// if this comment isn't a build constraint.
///
/// Handles `//`-line and `/* */`-block comments. Strips CRLF.
fn parse_build_tag(text: &str) -> Option<String> {
    let stripped = text.trim_end_matches('\r');
    // Strip comment markers.
    let body = if let Some(rest) = stripped.strip_prefix("//") {
        rest
    } else if let Some(rest) = stripped.strip_prefix("/*") {
        rest.strip_suffix("*/").unwrap_or(rest)
    } else {
        return None;
    };
    let body = body.trim();
    if let Some(expr) = body.strip_prefix("go:build ") {
        return Some(expr.trim().to_string());
    }
    if let Some(expr) = body.strip_prefix("+build ") {
        return Some(expr.trim().to_string());
    }
    None
}

/// Locate the deepest tree-sitter node spanning `[start_byte, end_byte]`.
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
    use super::super::queries::{compile_symbol_query, extract_symbols};
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> Vec<GoAttrsRow> {
        let mut parser = create_parser(Language::Go).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Go).expect("symbol query");
        let symbols = extract_symbols(&tree, src.as_bytes(), &query, path);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn exported_function_is_exported_no_receiver() {
        let rows = run("package main\nfunc Hello() {}", "main.go");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|Hello|"))
            .unwrap();
        assert!(r.is_exported);
        assert!(!r.has_receiver);
        assert!(r.build_tags.is_empty());
    }

    #[test]
    fn unexported_function_is_not_exported() {
        let rows = run("package main\nfunc hello() {}", "main.go");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|hello|"))
            .unwrap();
        assert!(!r.is_exported);
        assert!(!r.has_receiver);
    }

    #[test]
    fn method_has_receiver() {
        let rows = run(
            "package main\ntype Foo struct{}\nfunc (f *Foo) Bar() {}",
            "main.go",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|Bar|method"))
            .unwrap();
        assert!(r.is_exported);
        assert!(r.has_receiver);
    }

    #[test]
    fn struct_no_receiver() {
        let rows = run("package main\ntype Order struct { ID int }", "main.go");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|Order|struct"))
            .unwrap();
        assert!(r.is_exported);
        assert!(!r.has_receiver);
    }

    #[test]
    fn unexported_method_lowercase() {
        let rows = run(
            "package main\ntype Dispatcher struct{}\nfunc (d *Dispatcher) processOne() {}",
            "main.go",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|processOne|method"))
            .unwrap();
        assert!(!r.is_exported);
        assert!(r.has_receiver);
    }

    #[test]
    fn build_tag_modern_form() {
        let src = "//go:build linux && !arm\n\npackage worker\nfunc Cleanup() {}";
        let rows = run(src, "cleanup_linux.go");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.contains("|Cleanup|"))
            .unwrap();
        assert_eq!(r.build_tags, vec!["linux && !arm".to_string()]);
    }

    #[test]
    fn build_tag_both_forms_source_order() {
        let src = "//go:build linux && !arm\n// +build linux,!arm\n\npackage worker\nfunc F() {}";
        let rows = run(src, "f_linux.go");
        let r = rows.iter().find(|r| r.symbol_id.contains("|F|")).unwrap();
        assert_eq!(
            r.build_tags,
            vec!["linux && !arm".to_string(), "linux,!arm".to_string()]
        );
    }

    #[test]
    fn build_tags_broadcast_to_every_symbol() {
        let src = "//go:build linux\n\npackage worker\nfunc A() {}\nfunc B() {}";
        let rows = run(src, "x.go");
        let a = rows.iter().find(|r| r.symbol_id.contains("|A|")).unwrap();
        let b = rows.iter().find(|r| r.symbol_id.contains("|B|")).unwrap();
        assert_eq!(a.build_tags, vec!["linux".to_string()]);
        assert_eq!(b.build_tags, vec!["linux".to_string()]);
    }

    #[test]
    fn comment_after_package_is_not_build_tag() {
        let src = "package worker\n//go:build linux\nfunc A() {}";
        let rows = run(src, "x.go");
        let a = rows.iter().find(|r| r.symbol_id.contains("|A|")).unwrap();
        assert!(a.build_tags.is_empty());
    }

    #[test]
    fn one_row_per_symbol() {
        let rows = run(
            "package main\nfunc Alpha() {}\nfunc beta() {}\ntype S struct{}",
            "main.go",
        );
        // At minimum Alpha, beta, S — plus possibly more (fields, etc.)
        assert!(rows.len() >= 3, "got {}: {:?}", rows.len(), rows);
    }
}
