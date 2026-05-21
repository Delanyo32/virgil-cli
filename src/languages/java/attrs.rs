//! Issue #15 Java — `java_attrs` extractor.
//!
//! Implements the MVP columns from `docs/attrs-java.md`:
//! - `annotations`     — simple names of `@Foo` / `@Foo(...)` markers on
//!   the symbol's `modifiers` node, in source order.
//! - `is_final`        — `final` keyword in `modifiers`.
//! - `is_synchronized` — `synchronized` keyword in `modifiers` (method
//!   modifier form only; `synchronized (x) {}` statements do not count).
//! - `throws_clause`   — textual exception names from the `throws`
//!   clause on `method_declaration` / `constructor_declaration`.
//!
//! `is_native` / `is_default` / `type_parameters` from the contract are
//! deferred until the schema row gains those columns. We emit one row
//! per Java symbol regardless of whether any column is non-default.
//!
//! Symbol ID convention: ADR-0002 (`path|line|col|name|kind`).

use crate::models::{JavaAttrsRow, SymbolInfo};
use tree_sitter::{Node, Tree};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<JavaAttrsRow> {
    let mut out = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );

        let def_node = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte);

        let (annotations, is_final, is_synchronized) = match def_node {
            Some(n) => {
                let anns = collect_annotations(n, source);
                let f = has_modifier_keyword(n, source, "final");
                let s = has_modifier_keyword(n, source, "synchronized");
                (anns, f, s)
            }
            None => (Vec::new(), false, false),
        };

        let throws_clause = def_node
            .map(|n| collect_throws(n, source))
            .unwrap_or_default();

        out.push(JavaAttrsRow {
            symbol_id,
            annotations,
            is_final,
            is_synchronized,
            throws_clause,
        });
    }
    out
}

/// Walk the `modifiers` child (if any) and return the simple names of
/// every `marker_annotation` / `annotation` in source order.
fn collect_annotations(def_node: Node, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() != "modifiers" {
            continue;
        }
        let mut mod_cursor = child.walk();
        for modifier in child.children(&mut mod_cursor) {
            match modifier.kind() {
                "marker_annotation" | "annotation" => {
                    if let Some(name) = annotation_simple_name(modifier, source) {
                        out.push(name);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Extract the simple name from a `marker_annotation` / `annotation`
/// node. tree-sitter-java exposes a `name` field that can be either an
/// `identifier` or a `scoped_identifier`. For scoped names we want the
/// trailing identifier (e.g. `org.springframework.stereotype.Service` →
/// `Service`).
fn annotation_simple_name(ann_node: Node, source: &[u8]) -> Option<String> {
    let name_node = ann_node.child_by_field_name("name")?;
    let text = name_node.utf8_text(source).ok()?;
    let simple = text.rsplit('.').next().unwrap_or(text).trim();
    if simple.is_empty() {
        None
    } else {
        Some(simple.to_string())
    }
}

/// True if `def_node` carries the given keyword as a direct child of
/// its `modifiers` node.
fn has_modifier_keyword(def_node: Node, source: &[u8], keyword: &str) -> bool {
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() != "modifiers" {
            continue;
        }
        let mut mod_cursor = child.walk();
        for modifier in child.children(&mut mod_cursor) {
            if modifier.utf8_text(source).unwrap_or("") == keyword {
                return true;
            }
        }
    }
    false
}

/// Walk a method/constructor declaration's children for a `throws`
/// clause and return its textual exception type list. Whitespace is
/// collapsed to single spaces so multi-line declarations normalise.
fn collect_throws(def_node: Node, source: &[u8]) -> Vec<String> {
    if def_node.kind() != "method_declaration" && def_node.kind() != "constructor_declaration" {
        return Vec::new();
    }
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() != "throws" {
            continue;
        }
        let mut out = Vec::new();
        let mut tc = child.walk();
        for ty in child.children(&mut tc) {
            // Skip the `throws` keyword and comma punctuation.
            if !ty.is_named() {
                continue;
            }
            let raw = ty.utf8_text(source).unwrap_or("");
            let normalised = normalise_whitespace(raw);
            if !normalised.is_empty() {
                out.push(normalised);
            }
        }
        return out;
    }
    Vec::new()
}

fn normalise_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws && !out.is_empty() {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Locate the deepest tree-sitter node spanning exactly
/// `[start_byte, end_byte]`. Used to back-reference a `SymbolInfo` to
/// its AST definition node.
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
    use crate::languages::java::{compile_symbol_query, extract_symbols};
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> Vec<JavaAttrsRow> {
        let mut parser = create_parser(Language::Java).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Java).expect("symbol query");
        let symbols = extract_symbols(&tree, src.as_bytes(), &query, path);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn class_with_service_annotation() {
        let rows = run("@Service\npublic class ProductService { }", "Foo.java");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|ProductService|class"))
            .expect("class row");
        assert_eq!(r.annotations, vec!["Service".to_string()]);
        assert!(!r.is_final);
        assert!(!r.is_synchronized);
        assert!(r.throws_clause.is_empty());
    }

    #[test]
    fn multiple_annotations_preserve_order() {
        let rows = run(
            "@RestController\n@RequestMapping(\"/api\")\npublic class C { }",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|C|class"))
            .expect("class row");
        assert_eq!(
            r.annotations,
            vec!["RestController".to_string(), "RequestMapping".to_string()]
        );
    }

    #[test]
    fn scoped_annotation_returns_simple_name() {
        let rows = run(
            "@org.springframework.stereotype.Service\npublic class C { }",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|C|class"))
            .expect("class row");
        assert_eq!(r.annotations, vec!["Service".to_string()]);
    }

    #[test]
    fn annotation_arguments_dropped() {
        let rows = run(
            "public class C { @Cacheable(value = \"products\") public void f() {} }",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|f|method"))
            .expect("method row");
        assert_eq!(r.annotations, vec!["Cacheable".to_string()]);
    }

    #[test]
    fn final_field_marked() {
        let rows = run(
            "public class C { private static final int MAX = 100; }",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|MAX|variable"))
            .expect("field row");
        assert!(r.is_final);
        assert!(!r.is_synchronized);
    }

    #[test]
    fn synchronized_method_marked() {
        let rows = run(
            "public class C { public synchronized void inc() { } }",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|inc|method"))
            .expect("method row");
        assert!(r.is_synchronized);
        assert!(!r.is_final);
    }

    #[test]
    fn synchronized_statement_does_not_count() {
        // `synchronized (x) { ... }` is a statement, not a modifier.
        let rows = run(
            "public class C { Object x = new Object(); void f() { synchronized (x) { } } }",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|f|method"))
            .expect("method row");
        assert!(!r.is_synchronized);
    }

    #[test]
    fn throws_clause_extracted() {
        let rows = run(
            "public class C { void f() throws IOException, SQLException { } }",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|f|method"))
            .expect("method row");
        assert_eq!(
            r.throws_clause,
            vec!["IOException".to_string(), "SQLException".to_string()]
        );
    }

    #[test]
    fn throws_clause_multiline_normalised() {
        let rows = run(
            "public class C {\n  void f()\n      throws ServletException,\n             IOException { }\n}",
            "Foo.java",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|f|method"))
            .expect("method row");
        assert_eq!(
            r.throws_clause,
            vec!["ServletException".to_string(), "IOException".to_string()]
        );
    }

    #[test]
    fn one_row_per_symbol() {
        let rows = run(
            "public class C { void a() {} void b() {} int x; }",
            "Foo.java",
        );
        // C, a, b, x at minimum.
        assert!(rows.len() >= 4, "got {}: {:?}", rows.len(), rows);
    }

    #[test]
    fn non_method_symbol_has_empty_throws() {
        let rows = run("public class C { int x; }", "Foo.java");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|x|variable"))
            .expect("field row");
        assert!(r.throws_clause.is_empty());
    }
}
