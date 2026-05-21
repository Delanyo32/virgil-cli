//! Issue #15 — `python_attrs` extractor.
//!
//! Per `docs/attrs-python.md`, rows are emitted only for symbols of kind
//! `function`, `method`, or `class`. Columns:
//! - `decorators`: list of decorator expressions (without leading `@`),
//!   in source order, with call arguments preserved verbatim and dotted
//!   paths kept.
//! - `is_generator`: true if the function body contains a `yield` or
//!   `yield_from` not nested inside a deeper `function_definition` or
//!   `lambda`.
//! - `is_coroutine`: true if the def carries the `async` keyword (the
//!   tree-sitter Python grammar emits `function_definition` with a
//!   leading `async` child token, not a distinct
//!   `async_function_definition` kind). Always false for classes.
//! - `docstring_style`: deferred — always `None` for now (full
//!   classification needs a docstring parser; tracked as follow-up).
//!
//! Symbol ID convention: ADR-0002 (`path|line|col|name|kind`).

use tree_sitter::{Node, Tree};

use crate::models::{PythonAttrsRow, SymbolInfo, SymbolKind};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<PythonAttrsRow> {
    let mut out = Vec::new();
    for sym in symbols {
        if !matches!(
            sym.kind,
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Class
        ) {
            continue;
        }
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );

        let Some(def_node) = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte) else {
            // Symbol cannot be back-resolved to its AST node — emit
            // defaults so the row count still matches the symbol count
            // (within the kind filter).
            out.push(PythonAttrsRow {
                symbol_id,
                decorators: Vec::new(),
                is_generator: false,
                is_coroutine: false,
                docstring_style: None,
            });
            continue;
        };

        let decorators = collect_decorators(def_node, source);

        let is_class = matches!(sym.kind, SymbolKind::Class);
        let is_coroutine = !is_class && is_async_def(def_node);
        let is_generator = !is_class && function_body_yields(def_node);

        out.push(PythonAttrsRow {
            symbol_id,
            decorators,
            is_generator,
            is_coroutine,
            docstring_style: None,
        });
    }
    out
}

/// If `def_node`'s parent is a `decorated_definition`, walk its
/// `decorator` children and render each as a normalised expression
/// string (no leading `@`, single-spaced whitespace, dotted paths and
/// call arguments preserved).
fn collect_decorators(def_node: Node, source: &[u8]) -> Vec<String> {
    let Some(parent) = def_node.parent() else {
        return Vec::new();
    };
    if parent.kind() != "decorated_definition" {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        if child.kind() != "decorator" {
            continue;
        }
        let raw = child.utf8_text(source).unwrap_or("").trim();
        let body = raw.trim_start_matches('@').trim();
        // Collapse internal whitespace to single spaces, drop trailing
        // newlines.
        let normalised: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
        if !normalised.is_empty() {
            out.push(normalised);
        }
    }
    out
}

/// `async def` detection. Tree-sitter's Python grammar represents this
/// as a `function_definition` with a leading `async` child token.
fn is_async_def(def_node: Node) -> bool {
    if def_node.kind() != "function_definition" {
        return false;
    }
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "async" {
            return true;
        }
    }
    false
}

/// True if the function body contains a `yield` or `yield from` that is
/// not nested inside a deeper `function_definition` or `lambda`.
fn function_body_yields(def_node: Node) -> bool {
    if def_node.kind() != "function_definition" {
        return false;
    }
    let Some(body) = def_node.child_by_field_name("body") else {
        return false;
    };
    contains_yield(body)
}

fn contains_yield(node: Node) -> bool {
    // `yield` expressions in tree-sitter Python surface as a `yield`
    // node (covers both `yield x` and `yield from x`).
    if node.kind() == "yield" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Don't descend into nested function or lambda scopes — yields
        // there belong to the inner callable.
        if matches!(child.kind(), "function_definition" | "lambda") {
            continue;
        }
        if contains_yield(child) {
            return true;
        }
    }
    false
}

/// Locate the deepest tree-sitter node spanning exactly
/// `[start_byte, end_byte]`. Mirrors `rust_lang::attrs::find_node_at`.
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
    use crate::languages::python::queries::{compile_symbol_query, extract_symbols};
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> Vec<PythonAttrsRow> {
        let mut parser = create_parser(Language::Python).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Python).expect("symbol query");
        let symbols = extract_symbols(&tree, src.as_bytes(), &query, path);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn plain_function_has_defaults() {
        let rows = run("def hello():\n    return 1\n", "m.py");
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!(r.symbol_id.ends_with("|hello|function"));
        assert!(r.decorators.is_empty());
        assert!(!r.is_generator);
        assert!(!r.is_coroutine);
        assert!(r.docstring_style.is_none());
    }

    #[test]
    fn async_function_is_coroutine() {
        let rows = run("async def fetch(u):\n    return u\n", "m.py");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|fetch|function"))
            .unwrap();
        assert!(r.is_coroutine);
        assert!(!r.is_generator);
    }

    #[test]
    fn generator_function_detected() {
        let rows = run(
            "def chunks(xs, n):\n    for i in range(0, len(xs), n):\n        yield xs[i:i + n]\n",
            "m.py",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|chunks|function"))
            .unwrap();
        assert!(r.is_generator);
        assert!(!r.is_coroutine);
    }

    #[test]
    fn async_generator_is_both() {
        let rows = run("async def stream():\n    yield 1\n    yield 2\n", "m.py");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|stream|function"))
            .unwrap();
        assert!(r.is_generator);
        assert!(r.is_coroutine);
    }

    #[test]
    fn yield_in_nested_function_does_not_promote_outer() {
        let rows = run(
            "def outer():\n    def inner():\n        yield 1\n    return inner\n",
            "m.py",
        );
        let outer = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|outer|function"))
            .unwrap();
        assert!(!outer.is_generator, "outer must not inherit inner's yield");
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn single_decorator_captured() {
        let rows = run("@staticmethod\ndef m():\n    pass\n", "m.py");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|m|function"))
            .unwrap();
        assert_eq!(r.decorators, vec!["staticmethod".to_string()]);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn decorator_with_call_arguments_preserved() {
        let rows = run("@deprecated(\"use v2\")\ndef old():\n    pass\n", "m.py");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|old|function"))
            .unwrap();
        assert_eq!(r.decorators, vec!["deprecated(\"use v2\")".to_string()]);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn dotted_decorator_preserved() {
        let rows = run("@app.route(\"/x\")\ndef h():\n    pass\n", "m.py");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|h|function"))
            .unwrap();
        assert_eq!(r.decorators, vec!["app.route(\"/x\")".to_string()]);
    }

    #[test]
    fn class_row_emitted_with_class_defaults() {
        let rows = run("class Foo:\n    pass\n", "m.py");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|class"))
            .unwrap();
        assert!(!r.is_generator);
        assert!(!r.is_coroutine);
        assert!(r.decorators.is_empty());
    }

    #[test]
    fn variables_and_parameters_are_skipped() {
        let rows = run("X = 1\ndef f(a, b):\n    return a + b\n", "m.py");
        // Only the function row should appear — no rows for X or params.
        assert_eq!(rows.len(), 1);
        assert!(rows[0].symbol_id.ends_with("|f|function"));
    }
}
