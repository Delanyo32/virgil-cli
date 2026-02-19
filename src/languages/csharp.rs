use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──

const CSHARP_SYMBOL_QUERY: &str = r#"
(class_declaration
  name: (identifier) @name) @definition

(struct_declaration
  name: (identifier) @name) @definition

(interface_declaration
  name: (identifier) @name) @definition

(enum_declaration
  name: (identifier) @name) @definition

(record_declaration
  name: (identifier) @name) @definition

(method_declaration
  name: (identifier) @name) @definition

(constructor_declaration
  name: (identifier) @name) @definition

(namespace_declaration
  name: (identifier) @name) @definition

(namespace_declaration
  name: (qualified_name) @name) @definition

(property_declaration
  name: (identifier) @name) @definition

(delegate_declaration
  name: (identifier) @name) @definition

(field_declaration
  (variable_declaration
    (variable_declarator
      (identifier) @name))) @definition
"#;

// ── Import queries ──

const CSHARP_IMPORT_QUERY: &str = r#"
(using_directive) @import
"#;

// ── Comment queries ──

const COMMENT_QUERY: &str = r#"
(comment) @comment
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, CSHARP_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, CSHARP_IMPORT_QUERY)
        .with_context(|| format!("failed to compile import query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, COMMENT_QUERY)
        .with_context(|| format!("failed to compile comment query for {language}"))?;
    Ok(Arc::new(query))
}

// ── Symbol extraction ──

pub fn extract_symbols(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
) -> Vec<SymbolInfo> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let name_idx = query.capture_index_for_name("name");
    let definition_idx = query.capture_index_for_name("definition");

    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        let name_cap = name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let def_cap = definition_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));

        let (Some(name_cap), Some(def_cap)) = (name_cap, def_cap) else {
            continue;
        };

        let name_node = name_cap.node;
        let def_node = def_cap.node;

        let name = name_node.utf8_text(source).unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }

        let kind = determine_csharp_kind(def_node);
        let Some(kind) = kind else { continue };

        let is_exported = is_exported_csharp(def_node, source);

        let symbol = SymbolInfo {
            name,
            kind,
            file_path: file_path.to_string(),
            start_line: def_node.start_position().row as u32,
            start_column: def_node.start_position().column as u32,
            end_line: def_node.end_position().row as u32,
            end_column: def_node.end_position().column as u32,
            is_exported,
        };
        symbols.push(symbol);
    }

    symbols
}

fn determine_csharp_kind(def_node: tree_sitter::Node) -> Option<SymbolKind> {
    match def_node.kind() {
        "class_declaration" | "record_declaration" => Some(SymbolKind::Class),
        "struct_declaration" => Some(SymbolKind::Struct),
        "interface_declaration" => Some(SymbolKind::Interface),
        "enum_declaration" => Some(SymbolKind::Enum),
        "method_declaration" | "constructor_declaration" => Some(SymbolKind::Method),
        "namespace_declaration" => Some(SymbolKind::Namespace),
        "property_declaration" => Some(SymbolKind::Property),
        "delegate_declaration" => Some(SymbolKind::TypeAlias),
        "field_declaration" => Some(SymbolKind::Variable),
        _ => None,
    }
}

fn is_exported_csharp(def_node: tree_sitter::Node, source: &[u8]) -> bool {
    // Namespaces are always exported
    if def_node.kind() == "namespace_declaration" {
        return true;
    }

    // Check modifier children for access level
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "modifier" {
            let text = child.utf8_text(source).unwrap_or("");
            match text {
                "public" | "internal" => return true,
                "private" | "protected" => return false,
                _ => {}
            }
        }
    }

    false // conservative default: not exported
}

// ── Import extraction ──

pub fn extract_imports(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
) -> Vec<ImportInfo> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let import_idx = query.capture_index_for_name("import");

    let mut imports = Vec::new();

    while let Some(m) = matches.next() {
        let import_cap = import_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let Some(import_cap) = import_cap else {
            continue;
        };

        let node = import_cap.node;
        let text = node.utf8_text(source).unwrap_or("").to_string();

        let module_specifier = extract_using_namespace(&text);
        if module_specifier.is_empty() {
            continue;
        }

        imports.push(ImportInfo {
            source_file: file_path.to_string(),
            module_specifier,
            imported_name: "*".to_string(),
            local_name: "*".to_string(),
            kind: "using".to_string(),
            is_type_only: false,
            line: node.start_position().row as u32,
            is_external: true, // no syntactic way to distinguish
        });
    }

    imports
}

fn extract_using_namespace(text: &str) -> String {
    // Extract namespace from "using System.Collections.Generic;"
    let text = text.trim();
    let text = text.strip_prefix("using").unwrap_or(text).trim();
    let text = text.strip_prefix("static").unwrap_or(text).trim();
    let text = text.strip_suffix(';').unwrap_or(text).trim();

    // Handle alias: "using Alias = Namespace.Type" → take just the namespace part
    if let Some((_alias, ns)) = text.split_once('=') {
        return ns.trim().to_string();
    }

    text.to_string()
}

// ── Comment extraction ──

pub fn extract_comments(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
) -> Vec<CommentInfo> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let comment_idx = query.capture_index_for_name("comment");

    let mut comments = Vec::new();

    while let Some(m) = matches.next() {
        let comment_cap = comment_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let Some(comment_cap) = comment_cap else {
            continue;
        };

        let node = comment_cap.node;
        let text = node.utf8_text(source).unwrap_or("").to_string();
        if text.is_empty() {
            continue;
        }

        let kind = classify_comment(&text);
        let (associated_symbol, associated_symbol_kind) = find_associated_symbol(node, source);

        comments.push(CommentInfo {
            file_path: file_path.to_string(),
            text,
            kind,
            start_line: node.start_position().row as u32,
            start_column: node.start_position().column as u32,
            end_line: node.end_position().row as u32,
            end_column: node.end_position().column as u32,
            associated_symbol,
            associated_symbol_kind,
        });
    }

    comments
}

fn classify_comment(text: &str) -> String {
    let trimmed = text.trim_start();
    // C# XML doc comments use ///
    if trimmed.starts_with("///") {
        "doc".to_string()
    } else if trimmed.starts_with("/**") {
        "doc".to_string()
    } else if trimmed.starts_with("/*") {
        "block".to_string()
    } else {
        "line".to_string()
    }
}

fn find_associated_symbol(
    comment_node: tree_sitter::Node,
    source: &[u8],
) -> (Option<String>, Option<String>) {
    let sibling = comment_node.next_named_sibling();
    let Some(sibling) = sibling else {
        return (None, None);
    };

    extract_symbol_from_node(sibling, source)
}

fn extract_symbol_from_node(
    node: tree_sitter::Node,
    source: &[u8],
) -> (Option<String>, Option<String>) {
    match node.kind() {
        "class_declaration" | "record_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("class".to_string()))
        }
        "struct_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("struct".to_string()))
        }
        "interface_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("interface".to_string()))
        }
        "enum_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("enum".to_string()))
        }
        "method_declaration" | "constructor_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("method".to_string()))
        }
        "namespace_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("namespace".to_string()))
        }
        "property_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("property".to_string()))
        }
        "field_declaration" => {
            let name = extract_field_name(node, source);
            (name, Some("variable".to_string()))
        }
        "delegate_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("type_alias".to_string()))
        }
        _ => (None, None),
    }
}

fn extract_field_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Drill through variable_declaration → variable_declarator → identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declaration" {
            let mut inner_cursor = child.walk();
            for inner_child in child.children(&mut inner_cursor) {
                if inner_child.kind() == "variable_declarator" {
                    let mut var_cursor = inner_child.walk();
                    for var_child in inner_child.children(&mut var_cursor) {
                        if var_child.kind() == "identifier" {
                            return var_child.utf8_text(source).ok().map(|s| s.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::create_parser;

    fn parse_and_extract(source: &str) -> Vec<SymbolInfo> {
        let mut parser = create_parser(Language::CSharp).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::CSharp).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.cs")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::CSharp).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::CSharp).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.cs")
    }

    #[test]
    fn extract_class() {
        let syms = parse_and_extract("public class Foo { }");
        let s = syms.iter().find(|s| s.name == "Foo");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Class);
        assert!(s.unwrap().is_exported);
    }

    #[test]
    fn extract_private_class() {
        let syms = parse_and_extract("private class Foo { }");
        let s = syms.iter().find(|s| s.name == "Foo");
        assert!(s.is_some());
        assert!(!s.unwrap().is_exported);
    }

    #[test]
    fn extract_struct() {
        let syms = parse_and_extract("public struct Point { }");
        let s = syms.iter().find(|s| s.name == "Point");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn extract_interface() {
        let syms = parse_and_extract("public interface IFoo { }");
        let s = syms.iter().find(|s| s.name == "IFoo");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn extract_enum() {
        let syms = parse_and_extract("public enum Color { Red, Green, Blue }");
        let s = syms.iter().find(|s| s.name == "Color");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn extract_namespace() {
        let syms = parse_and_extract("namespace MyApp { }");
        let s = syms.iter().find(|s| s.name == "MyApp");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Namespace);
        assert!(s.unwrap().is_exported); // namespaces always exported
    }

    #[test]
    fn extract_method() {
        let syms = parse_and_extract("public class Foo { public void Bar() { } }");
        let method = syms.iter().find(|s| s.name == "Bar");
        assert!(method.is_some());
        assert_eq!(method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn using_directive() {
        let imports = parse_and_extract_imports("using System;\nusing System.Collections.Generic;");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module_specifier, "System");
        assert_eq!(imports[0].kind, "using");
        assert!(imports[0].is_external);
    }

    #[test]
    fn extract_using_namespace_helper() {
        assert_eq!(extract_using_namespace("using System;"), "System");
        assert_eq!(
            extract_using_namespace("using System.Collections.Generic;"),
            "System.Collections.Generic"
        );
        assert_eq!(
            extract_using_namespace("using static System.Math;"),
            "System.Math"
        );
        assert_eq!(
            extract_using_namespace("using Console = System.Console;"),
            "System.Console"
        );
    }

    #[test]
    fn comment_classification() {
        assert_eq!(classify_comment("/// XML doc"), "doc");
        assert_eq!(classify_comment("/** Javadoc style */"), "doc");
        assert_eq!(classify_comment("/* block */"), "block");
        assert_eq!(classify_comment("// line"), "line");
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("");
        assert!(syms.is_empty());
    }
}
