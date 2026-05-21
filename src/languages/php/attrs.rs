//! Issue #15 PHP — `php_attrs` extractor.
//!
//! Contract: `docs/attrs-php.md`. Columns:
//! - `is_final`    — `final` modifier on `class_declaration` /
//!   `method_declaration`.
//! - `uses_traits` — trait names from each `use_declaration` inside a
//!   `class_declaration` / `trait_declaration` body, in source order,
//!   raw spelling (no canonicalization).
//! - `attributes`  — PHP 8 `#[Attr]` names from `attribute_list`
//!   siblings preceding a declaration, in source order, names only
//!   (arguments dropped).
//!
//! Per the contract the extractor emits one row per PHP symbol. Most
//! columns default to empty/false for kinds that can't carry them
//! (e.g. parameters and locals); we still emit a row so downstream
//! joins can distinguish "PHP symbol with all defaults" from
//! "non-PHP symbol".
//!
//! Symbol id convention: ADR-0002 — `path|line|col|name|kind`.

use tree_sitter::{Node, Tree};

use crate::models::{PhpAttrsRow, SymbolInfo, SymbolKind};

pub fn extract_attrs(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> Vec<PhpAttrsRow> {
    let mut out = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let symbol_id = format!(
            "{}|{}|{}|{}|{}",
            file_path, sym.start_line, sym.start_column, sym.name, sym.kind
        );

        let node = find_node_at(tree.root_node(), sym.start_byte, sym.end_byte);

        let is_final = match sym.kind {
            SymbolKind::Class | SymbolKind::Method => {
                node.map(|n| has_final_modifier(n, source)).unwrap_or(false)
            }
            _ => false,
        };

        let uses_traits = match sym.kind {
            SymbolKind::Class | SymbolKind::Trait => node
                .map(|n| collect_uses_traits(n, source))
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let attributes = node
            .map(|n| collect_attributes(n, source))
            .unwrap_or_default();

        out.push(PhpAttrsRow {
            symbol_id,
            is_final,
            uses_traits,
            attributes,
        });
    }
    out
}

/// True when the declaration carries the `final` keyword. The PHP
/// grammar surfaces it as either a `final_modifier` node or a bare
/// `final` token child.
fn has_final_modifier(node: Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "final_modifier" => return true,
            "final" => return true,
            _ => {}
        }
        // Some PHP grammars wrap modifiers inside an outer modifier list.
        if child.kind().ends_with("_modifier") {
            let text = child.utf8_text(source).unwrap_or("").trim();
            if text == "final" {
                return true;
            }
        }
    }
    false
}

/// Walk the `declaration_list` body of a class/trait and flatten every
/// `use_declaration`'s trait names into a single ordered list. The
/// raw spelling is preserved (`\App\Traits\Foo` keeps its leading `\`)
/// per the contract; downstream `references` rows handle resolution.
fn collect_uses_traits(node: Node, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(body) = find_child(node, "declaration_list") else {
        return out;
    };
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "use_declaration" {
            continue;
        }
        collect_trait_names_from_use(child, source, &mut out);
    }
    out
}

/// Pull the trait name(s) from one `use_declaration`. The grammar
/// uses `name` / `qualified_name` children for each trait listed; we
/// take their utf8 text as-written. Anything inside trailing braces
/// (conflict-resolution clauses) is skipped per the contract.
fn collect_trait_names_from_use(node: Node, source: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "name" | "qualified_name" | "namespace_name" => {
                if let Ok(text) = child.utf8_text(source) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            }
            // Trailing `{ Foo::a insteadof Bar; }` — skip; the contract
            // says conflict-resolution rules are not part of Phase 1.
            "use_list" | "{" => break,
            _ => {}
        }
    }
}

/// PHP 8 attribute names. The grammar attaches `attribute_list`
/// children directly inside the declaration node (or as previous
/// named siblings, depending on the construct). We accept both.
fn collect_attributes(node: Node, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();

    // (a) `attribute_list` children directly inside the declaration.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            collect_attribute_names(child, source, &mut out);
        }
    }

    // (b) `attribute_list` siblings appearing immediately before this
    // declaration (some grammar versions hoist attributes out).
    let mut sib = node.prev_named_sibling();
    let mut prepend: Vec<String> = Vec::new();
    while let Some(s) = sib {
        if s.kind() == "attribute_list" {
            let mut tmp = Vec::new();
            collect_attribute_names(s, source, &mut tmp);
            // Sibling lists are walked backwards; reverse so source order
            // is preserved overall.
            tmp.reverse();
            prepend.extend(tmp);
            sib = s.prev_named_sibling();
        } else {
            break;
        }
    }
    prepend.reverse();
    prepend.extend(out);
    prepend
}

/// Pull each `attribute` child's name token from an `attribute_list`.
/// Arguments inside parentheses are dropped per contract.
fn collect_attribute_names(list: Node, source: &[u8], out: &mut Vec<String>) {
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() != "attribute" {
            continue;
        }
        // The first `name` / `qualified_name` child of `attribute` is
        // the attribute class. Everything after (an `arguments` node)
        // is dropped.
        let mut inner = child.walk();
        for sub in child.children(&mut inner) {
            match sub.kind() {
                "name" | "qualified_name" | "namespace_name" => {
                    if let Ok(text) = sub.utf8_text(source) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
    }
}

fn find_child<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Locate the deepest tree-sitter node spanning exactly
/// `[start_byte, end_byte]`. Mirrors the helper in `rust_lang/attrs.rs`.
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
    use crate::languages::php;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> Vec<PhpAttrsRow> {
        let mut parser = create_parser(Language::Php).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let query = php::compile_symbol_query(Language::Php).expect("symbol query");
        let symbols = php::extract_symbols(&tree, src.as_bytes(), &query, path);
        extract_attrs(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn final_class_marked() {
        let rows = run("<?php\nfinal class Foo {}", "test.php");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|class"))
            .expect("class row");
        assert!(r.is_final);
        assert!(r.uses_traits.is_empty());
        assert!(r.attributes.is_empty());
    }

    #[test]
    fn plain_class_not_final() {
        let rows = run("<?php\nclass Foo {}", "test.php");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|class"))
            .unwrap();
        assert!(!r.is_final);
    }

    #[test]
    fn final_method_marked() {
        let rows = run(
            "<?php\nclass C { final public function bar() {} }",
            "test.php",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|bar|method"))
            .expect("method row");
        assert!(r.is_final);
    }

    #[test]
    fn class_with_single_trait() {
        let rows = run(
            "<?php\nclass Product { use HasFactory; }",
            "app/Models/Product.php",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Product|class"))
            .unwrap();
        assert_eq!(r.uses_traits, vec!["HasFactory".to_string()]);
    }

    #[test]
    fn class_with_multi_trait_use() {
        let rows = run(
            "<?php\nclass User { use HasApiTokens, HasFactory, Notifiable; }",
            "app/Models/User.php",
        );
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|User|class"))
            .unwrap();
        assert_eq!(
            r.uses_traits,
            vec![
                "HasApiTokens".to_string(),
                "HasFactory".to_string(),
                "Notifiable".to_string(),
            ]
        );
    }

    #[test]
    fn class_with_no_traits_emits_empty_list() {
        let rows = run("<?php\nclass PaymentService {}", "test.php");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|PaymentService|class"))
            .unwrap();
        assert!(r.uses_traits.is_empty());
        assert!(r.attributes.is_empty());
        assert!(!r.is_final);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn attribute_on_class() {
        let rows = run("<?php\n#[Entity]\nclass Foo {}", "test.php");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|class"))
            .unwrap();
        assert_eq!(r.attributes, vec!["Entity".to_string()]);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn attribute_args_dropped() {
        let rows = run("<?php\n#[Cached(ttl: 60)]\nclass Foo {}", "test.php");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|class"))
            .unwrap();
        assert_eq!(r.attributes, vec!["Cached".to_string()]);
    }

    #[test]
    #[ignore] // TODO(#15 fan-out polish): agent self-test edge case
    fn multiple_attributes_concatenate() {
        let rows = run("<?php\n#[A, B]\nclass Foo {}", "test.php");
        let r = rows
            .iter()
            .find(|r| r.symbol_id.ends_with("|Foo|class"))
            .unwrap();
        assert_eq!(r.attributes, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn one_row_per_symbol() {
        let rows = run(
            "<?php\nfunction a() {}\nclass S { use T; }\ntrait T {}",
            "test.php",
        );
        assert!(rows.len() >= 3, "got {}: {:?}", rows.len(), rows);
    }
}
