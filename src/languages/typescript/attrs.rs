//! Issue #15 TypeScript/JavaScript `typescript_attrs` extractor.
//!
//! Contract: `docs/attrs-typescript.md`. Columns:
//! - `is_readonly` — `readonly` modifier on a field/parameter, or a
//!   `readonly_type` RHS on a `type_alias` / `variable` binding.
//! - `is_optional` — `?` suffix on a parameter (`optional_parameter`)
//!   or a property signature (`property_signature` with `?` token).
//! - `type_parameters` — declared type-parameter names (source order)
//!   from a `type_parameters` AST node on
//!   function/method/class/interface/type_alias declarations.
//!
//! We emit one row per TS/JS symbol (mirror Rust pilot). JavaScript
//! files emit defaults only — JS has no `readonly`/`?`/generics syntax.
//!
//! Symbol ID convention: ADR-0002 (`path|line|col|name|kind`).

use tree_sitter::{Node, Tree};

use crate::models::{SymbolInfo, SymbolKind, TypescriptAttrsRow};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<TypescriptAttrsRow> {
    let is_js = is_javascript_path(file_path);
    let mut out = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );
        if is_js {
            out.push(TypescriptAttrsRow {
                symbol_id,
                is_readonly: false,
                is_optional: false,
                type_parameters: Vec::new(),
            });
            continue;
        }
        let node = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte);
        let is_readonly = node.map(|n| node_is_readonly(n, sym)).unwrap_or(false);
        let is_optional = node.map(|n| node_is_optional(n, sym)).unwrap_or(false);
        let type_parameters = node
            .map(|n| collect_type_parameters(n, source))
            .unwrap_or_default();
        out.push(TypescriptAttrsRow {
            symbol_id,
            is_readonly,
            is_optional,
            type_parameters,
        });
    }
    out
}

/// Return true if the symbol's binding has the `readonly` modifier
/// (field / parameter-property) or its RHS is a `readonly_type`
/// (type_alias / variable binding).
fn node_is_readonly(node: Node, sym: &SymbolInfo) -> bool {
    match sym.kind {
        SymbolKind::TypeAlias => {
            if let Some(value) = node.child_by_field_name("value") {
                return has_readonly_type(value);
            }
            false
        }
        SymbolKind::Variable => {
            // `node` is typically the symbol-name node or the
            // variable_declarator; walk up to find the declarator.
            if let Some(declarator) = enclosing_kind(node, "variable_declarator")
                && let Some(t) = declarator.child_by_field_name("type")
            {
                let inner = unwrap_type_annotation(t);
                return has_readonly_type(inner);
            }
            false
        }
        _ => {
            // Field-like / parameter-property: scan for a `readonly` child token.
            has_readonly_modifier(node)
        }
    }
}

/// Return true if the node represents an optional parameter
/// (`optional_parameter`) or an optional property signature
/// (`property_signature` with a `?` token child).
fn node_is_optional(node: Node, _sym: &SymbolInfo) -> bool {
    match node.kind() {
        "optional_parameter" => true,
        "property_signature" | "public_field_definition" => {
            // Search for a literal "?" token among children.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "?" {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// Collect the names of all type parameters declared *directly* on
/// `node` (not on ancestors). Returns names in source order.
fn collect_type_parameters(node: Node, source: &[u8]) -> Vec<String> {
    let Some(tp) = node.child_by_field_name("type_parameters") else {
        // Fallback: scan named children for a `type_parameters` node.
        // Some grammar versions don't expose this via a field name on
        // every declaration kind.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "type_parameters" {
                return parse_type_parameters(child, source);
            }
        }
        return Vec::new();
    };
    parse_type_parameters(tp, source)
}

fn parse_type_parameters(tp: Node, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = tp.walk();
    for child in tp.named_children(&mut cursor) {
        if child.kind() == "type_parameter" {
            if let Some(name) = child.child_by_field_name("name")
                && let Ok(text) = name.utf8_text(source)
            {
                out.push(text.to_string());
                continue;
            }
            // Fallback: first type_identifier child.
            let mut inner = child.walk();
            for c in child.named_children(&mut inner) {
                if c.kind() == "type_identifier" {
                    if let Ok(text) = c.utf8_text(source) {
                        out.push(text.to_string());
                    }
                    break;
                }
            }
        }
    }
    out
}

fn has_readonly_modifier(node: Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if (child.kind() == "readonly" || child.kind() == "accessibility_modifier")
            && child.kind() == "readonly"
        {
            return true;
        }
        // Some grammars expose the keyword as an anonymous token.
        if child.kind() == "readonly" {
            return true;
        }
    }
    false
}

fn has_readonly_type(node: Node) -> bool {
    if node.kind() == "readonly_type" {
        return true;
    }
    if node.kind() == "parenthesized_type"
        && let Some(inner) = node.named_child(0)
    {
        return has_readonly_type(inner);
    }
    false
}

fn unwrap_type_annotation(node: Node) -> Node {
    if node.kind() == "type_annotation"
        && let Some(inner) = node.named_child(0)
    {
        return inner;
    }
    node
}

fn enclosing_kind<'a>(mut node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    loop {
        if node.kind() == kind {
            return Some(node);
        }
        node = node.parent()?;
    }
}

fn is_javascript_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
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
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str, lang: Language) -> Vec<TypescriptAttrsRow> {
        let mut parser = create_parser(lang).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = languages::compile_symbol_query(lang).expect("symbol query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &query, path, lang);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn generic_function_captures_type_param() {
        let rows = run(
            "export function useDebounce<T>(value: T, delay: number): T { return value; }",
            "src/useDebounce.ts",
            Language::TypeScript,
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|useDebounce|function"))
            .expect("useDebounce row");
        assert_eq!(r.type_parameters, vec!["T".to_string()]);
        assert!(!r.is_readonly);
        assert!(!r.is_optional);
    }

    #[test]
    fn multi_type_params_strip_bounds() {
        let rows = run(
            "function foo<T, U extends number>(x: T, y: U): T { return x; }",
            "src/foo.ts",
            Language::TypeScript,
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|foo|function"))
            .expect("foo row");
        assert_eq!(r.type_parameters, vec!["T".to_string(), "U".to_string()]);
    }

    #[test]
    fn generic_interface_captures_type_param() {
        let rows = run(
            "export interface PaginatedResponse<T> { data: T[]; total: number; }",
            "src/types.ts",
            Language::TypeScript,
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|PaginatedResponse|interface"))
            .expect("interface row");
        assert_eq!(r.type_parameters, vec!["T".to_string()]);
    }

    #[test]
    fn generic_class_and_method_independent() {
        let rows = run(
            "class Container<T> { map<U>(fn: (x: T) => U): Container<U> { return this as any; } }",
            "src/c.ts",
            Language::TypeScript,
        );
        let cls = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Container|class"))
            .expect("class row");
        assert_eq!(cls.type_parameters, vec!["T".to_string()]);
        let m = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|map|method"))
            .expect("method row");
        // Method records only its own type parameters; class's T does not flow in.
        assert_eq!(m.type_parameters, vec!["U".to_string()]);
    }

    #[test]
    fn type_alias_with_readonly_rhs() {
        let rows = run(
            "type ReadonlyIds = readonly number[];",
            "example.ts",
            Language::TypeScript,
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|ReadonlyIds|type_alias"))
            .expect("type_alias row");
        assert!(r.is_readonly);
        assert!(r.type_parameters.is_empty());
    }

    #[test]
    fn js_symbols_get_defaults() {
        let rows = run(
            "function authenticate(req, res, next) {}",
            "src/middleware/auth.js",
            Language::JavaScript,
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|authenticate|function"))
            .expect("js function row");
        assert!(!r.is_readonly);
        assert!(!r.is_optional);
        assert!(r.type_parameters.is_empty());
    }

    #[test]
    fn one_row_per_symbol() {
        let rows = run(
            "function alpha() {}\nfunction beta() {}\nclass S {}",
            "src/lib.ts",
            Language::TypeScript,
        );
        // alpha, beta, S — at minimum 3 rows.
        assert!(rows.len() >= 3, "got {}: {:?}", rows.len(), rows);
    }

    #[test]
    fn plain_function_has_empty_type_params() {
        let rows = run(
            "function plain(x: number): number { return x; }",
            "src/p.ts",
            Language::TypeScript,
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|plain|function"))
            .expect("plain row");
        assert!(r.type_parameters.is_empty());
    }
}
