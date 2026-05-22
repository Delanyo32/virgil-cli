use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind, SymbolVisibility};

/// Visibility for C symbols, per `docs/attrs-c.md`.
///
/// - Parameters and block-scope locals are always `Private`.
/// - File-scope declarations carrying `static` storage class get `Private`
///   (translation-unit local linkage).
/// - Everything else is `Public`. C has no scoped visibility keywords.
fn visibility_c(def_node: tree_sitter::Node, source: &[u8]) -> SymbolVisibility {
    if def_node.kind() == "parameter_declaration" {
        return SymbolVisibility::Private;
    }
    if is_block_scope(def_node) {
        return SymbolVisibility::Private;
    }
    if has_storage_class(def_node, source, "static") {
        return SymbolVisibility::Private;
    }
    SymbolVisibility::Public
}

/// True if `def_node` is nested inside a function body — i.e. block scope
/// rather than file scope. Used to classify locals as Private regardless
/// of their storage class.
fn is_block_scope(def_node: tree_sitter::Node) -> bool {
    let mut current = def_node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "compound_statement" | "function_definition" => return true,
            _ => current = parent.parent(),
        }
    }
    false
}

/// True if `def_node` has a direct child `storage_class_specifier` whose
/// text matches `keyword` (e.g. `"static"`, `"extern"`).
fn has_storage_class(def_node: tree_sitter::Node, source: &[u8], keyword: &str) -> bool {
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "storage_class_specifier"
            && child.utf8_text(source).unwrap_or("") == keyword
        {
            return true;
        }
    }
    false
}

// ── Symbol queries ──

const C_SYMBOL_QUERY: &str = r#"
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition

(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @name))) @definition

(declaration
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition

(declaration
  declarator: (init_declarator
    declarator: (identifier) @name)) @definition

(declaration
  declarator: (identifier) @name) @definition

(struct_specifier
  name: (type_identifier) @name
  body: (field_declaration_list)) @definition

(union_specifier
  name: (type_identifier) @name
  body: (field_declaration_list)) @definition

(enum_specifier
  name: (type_identifier) @name
  body: (enumerator_list)) @definition

(type_definition
  declarator: (type_identifier) @name) @definition

(preproc_def
  name: (identifier) @name) @definition

(preproc_function_def
  name: (identifier) @name) @definition

(parameter_declaration
  declarator: (identifier) @name) @definition

(parameter_declaration
  declarator: (pointer_declarator
    declarator: (identifier) @name)) @definition

(parameter_declaration
  declarator: (pointer_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @name))) @definition

(parameter_declaration
  declarator: (array_declarator
    declarator: (identifier) @name)) @definition

(declaration
  declarator: (pointer_declarator
    declarator: (identifier) @name)) @definition

(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @name))) @definition

(declaration
  declarator: (array_declarator
    declarator: (identifier) @name)) @definition

(field_declaration
  declarator: (field_identifier) @name) @definition

(field_declaration
  declarator: (pointer_declarator
    declarator: (field_identifier) @name)) @definition

(field_declaration
  declarator: (array_declarator
    declarator: (field_identifier) @name)) @definition
"#;

// ── Import queries ──

const C_IMPORT_QUERY: &str = r#"
(preproc_include
  path: (_) @path) @include
"#;

// ── Comment queries ──

const COMMENT_QUERY: &str = r#"
(comment) @comment
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, C_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, C_IMPORT_QUERY)
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

        let kind = determine_c_kind(def_node);
        let Some(kind) = kind else { continue };

        let is_exported = is_exported_c(def_node, source);
        let visibility = visibility_c(def_node, source);
        // `static` storage class — applies at both file scope and block
        // scope. Parameters cannot carry storage class specifiers.
        let is_static = matches!(def_node.kind(), "function_definition" | "declaration")
            && has_storage_class(def_node, source, "static");

        let symbol = SymbolInfo {
            name,
            kind,
            file_path: file_path.to_string(),
            start_byte: def_node.start_byte() as u32,
            end_byte: def_node.end_byte() as u32,
            start_line: def_node.start_position().row as u32 + 1,
            start_column: def_node.start_position().column as u32,
            end_line: def_node.end_position().row as u32 + 1,
            end_column: def_node.end_position().column as u32,
            is_exported,
            visibility,
            // C has no async concept.
            is_async: false,
            is_static,
            // C has no abstract concept.
            is_abstract: false,
            // C `const`-ness is tracked in `c_attrs.is_const`, not here.
            is_mutable: false,
        };
        symbols.push(symbol);
    }

    symbols
}

fn determine_c_kind(def_node: tree_sitter::Node) -> Option<SymbolKind> {
    match def_node.kind() {
        "function_definition" => Some(SymbolKind::Function),
        "declaration" => {
            // Check children to distinguish function prototype from variable
            let mut cursor = def_node.walk();
            for child in def_node.children(&mut cursor) {
                if child.kind() == "function_declarator" {
                    return Some(SymbolKind::Function);
                }
            }
            Some(SymbolKind::Variable)
        }
        "parameter_declaration" => Some(SymbolKind::Parameter),
        "struct_specifier" => Some(SymbolKind::Struct),
        "union_specifier" => Some(SymbolKind::Union),
        "enum_specifier" => Some(SymbolKind::Enum),
        "type_definition" => Some(SymbolKind::Typedef),
        "preproc_def" | "preproc_function_def" => Some(SymbolKind::Macro),
        "field_declaration" => Some(SymbolKind::Field),
        _ => None,
    }
}

fn is_exported_c(def_node: tree_sitter::Node, source: &[u8]) -> bool {
    // Macros, structs, unions, enums, and typedefs are always "exported"
    match def_node.kind() {
        "preproc_def"
        | "preproc_function_def"
        | "struct_specifier"
        | "union_specifier"
        | "enum_specifier"
        | "type_definition" => return true,
        _ => {}
    }

    // For functions and variables, check for `static` storage class
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "storage_class_specifier"
            && child.utf8_text(source).unwrap_or("") == "static"
        {
            return false;
        }
    }

    true // external linkage by default
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

    let path_idx = query.capture_index_for_name("path");
    let include_idx = query.capture_index_for_name("include");

    let mut imports = Vec::new();

    while let Some(m) = matches.next() {
        let path_cap = path_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let include_cap = include_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));

        let (Some(path_cap), Some(include_cap)) = (path_cap, include_cap) else {
            continue;
        };

        let path_node = path_cap.node;
        let include_node = include_cap.node;

        let raw_path = path_node.utf8_text(source).unwrap_or("").to_string();
        if raw_path.is_empty() {
            continue;
        }

        let is_system = path_node.kind() == "system_lib_string";
        let module_specifier = strip_include_path(&raw_path);

        imports.push(ImportInfo {
            source_file: file_path.to_string(),
            module_specifier,
            imported_name: "*".to_string(),
            local_name: "*".to_string(),
            kind: "include".to_string(),
            is_type_only: false,
            line: include_node.start_position().row as u32 + 1,
            is_external: is_system,
        });
    }

    imports
}

fn strip_include_path(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('<') && s.ends_with('>')) || (s.starts_with('"') && s.ends_with('"')) {
        s[1..s.len() - 1].trim().to_string()
    } else {
        s.to_string()
    }
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
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            start_line: node.start_position().row as u32 + 1,
            start_column: node.start_position().column as u32,
            end_line: node.end_position().row as u32 + 1,
            end_column: node.end_position().column as u32,
            associated_symbol,
            associated_symbol_kind,
        });
    }

    comments
}

fn classify_comment(text: &str) -> String {
    let trimmed = text.trim_start();
    if trimmed.starts_with("/**") || trimmed.starts_with("///") {
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
        "function_definition" => {
            let name = extract_function_name(node, source);
            (name, Some("function".to_string()))
        }
        "declaration" => {
            let has_func = has_child_kind(node, "function_declarator");
            let kind_str = if has_func { "function" } else { "variable" };
            let name = extract_declaration_name(node, source);
            (name, Some(kind_str.to_string()))
        }
        "struct_specifier" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("struct".to_string()))
        }
        "union_specifier" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("union".to_string()))
        }
        "enum_specifier" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("enum".to_string()))
        }
        "type_definition" => {
            let name = extract_typedef_name(node, source);
            (name, Some("typedef".to_string()))
        }
        "preproc_def" | "preproc_function_def" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("macro".to_string()))
        }
        _ => (None, None),
    }
}

fn extract_function_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    find_identifier_recursive(declarator, source)
}

fn extract_declaration_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    find_identifier_recursive(declarator, source)
}

fn extract_typedef_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    declarator.utf8_text(source).ok().map(|s| s.to_string())
}

fn find_identifier_recursive(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return node.utf8_text(source).ok().map(|s| s.to_string());
    }
    // Drill into declarator children
    if let Some(inner) = node.child_by_field_name("declarator") {
        return find_identifier_recursive(inner, source);
    }
    None
}

fn has_child_kind(node: tree_sitter::Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

// ── Import resolution ──

/// Resolve a C `#include "header.h"` to a file path.
/// Tries relative to source file's directory, then project root.
pub fn resolve_import(
    source_file: &str,
    specifier: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    let base_dir = source_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    // Try relative to source file's directory
    let relative = if base_dir.is_empty() {
        specifier.to_string()
    } else {
        format!("{}/{}", base_dir, specifier)
    };
    if known_files.contains(&relative) {
        return Some(relative);
    }

    // Try from project root
    if known_files.contains(specifier) {
        return Some(specifier.to_string());
    }

    None
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::create_parser;

    fn parse_and_extract(source: &str) -> Vec<SymbolInfo> {
        let mut parser = create_parser(Language::C).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::C).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.c")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::C).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::C).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.c")
    }

    fn parse_and_extract_comments(source: &str) -> Vec<CommentInfo> {
        let mut parser = create_parser(Language::C).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_comment_query(Language::C).expect("compile comment query");
        extract_comments(&tree, source.as_bytes(), &query, "test.c")
    }

    #[test]
    fn extract_function_definition() {
        let syms = parse_and_extract("int main(int argc, char **argv) { return 0; }");
        let main = syms
            .iter()
            .find(|s| s.name == "main")
            .expect("main missing");
        assert_eq!(main.kind, SymbolKind::Function);
        assert!(main.is_exported);
    }

    #[test]
    fn extract_static_function() {
        let syms = parse_and_extract("static void helper() { }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "helper");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(!syms[0].is_exported);
    }

    #[test]
    fn extract_struct() {
        let syms = parse_and_extract("struct Point { int x; int y; };");
        let s = syms.iter().find(|s| s.name == "Point");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn extract_enum() {
        let syms = parse_and_extract("enum Color { RED, GREEN, BLUE };");
        let s = syms.iter().find(|s| s.name == "Color");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn extract_typedef() {
        let syms = parse_and_extract("typedef unsigned int uint;");
        let s = syms.iter().find(|s| s.name == "uint");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Typedef);
    }

    #[test]
    fn extract_macro() {
        let syms = parse_and_extract("#define MAX_SIZE 100");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MAX_SIZE");
        assert_eq!(syms[0].kind, SymbolKind::Macro);
    }

    #[test]
    fn extract_macro_function() {
        let syms = parse_and_extract("#define ADD(a, b) ((a) + (b))");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "ADD");
        assert_eq!(syms[0].kind, SymbolKind::Macro);
    }

    #[test]
    fn extract_variable_with_init() {
        let syms = parse_and_extract("int count = 0;");
        let s = syms.iter().find(|s| s.name == "count");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Variable);
    }

    #[test]
    fn system_include() {
        let imports = parse_and_extract_imports("#include <stdio.h>");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "stdio.h");
        assert_eq!(imports[0].kind, "include");
        assert!(imports[0].is_external);
    }

    #[test]
    fn local_include() {
        let imports = parse_and_extract_imports("#include \"myheader.h\"");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "myheader.h");
        assert_eq!(imports[0].kind, "include");
        assert!(!imports[0].is_external);
    }

    #[test]
    fn comment_classification() {
        let comments =
            parse_and_extract_comments("/** Doc comment */\n// Line comment\n/* Block comment */");
        assert_eq!(comments.len(), 3);
        assert_eq!(comments[0].kind, "doc");
        assert_eq!(comments[1].kind, "line");
        assert_eq!(comments[2].kind, "block");
    }

    #[test]
    fn triple_slash_doc_comment() {
        let comments =
            parse_and_extract_comments("/// This is a doc comment\nint foo() { return 0; }");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, "doc");
    }

    #[test]
    fn comment_associated_symbol() {
        let comments = parse_and_extract_comments(
            "/** Calculate sum */\nint sum(int a, int b) { return a + b; }",
        );
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].associated_symbol.as_deref(), Some("sum"));
        assert_eq!(
            comments[0].associated_symbol_kind.as_deref(),
            Some("function")
        );
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("");
        assert!(syms.is_empty());
    }

    #[test]
    fn extract_function_parameters() {
        let syms = parse_and_extract("int add(int a, int b) { return a + b; }");
        let a = syms.iter().find(|s| s.name == "a");
        let b = syms.iter().find(|s| s.name == "b");
        assert!(a.is_some(), "parameter `a` missing");
        assert!(b.is_some(), "parameter `b` missing");
        assert_eq!(a.unwrap().kind, SymbolKind::Parameter);
        assert_eq!(b.unwrap().kind, SymbolKind::Parameter);
    }

    #[test]
    fn extract_pointer_parameter() {
        let syms = parse_and_extract("void set(int *p, char **argv) { }");
        let p = syms.iter().find(|s| s.name == "p");
        let argv = syms.iter().find(|s| s.name == "argv");
        assert!(p.is_some(), "pointer parameter `p` missing");
        assert!(argv.is_some(), "double-pointer parameter `argv` missing");
        assert_eq!(p.unwrap().kind, SymbolKind::Parameter);
        assert_eq!(argv.unwrap().kind, SymbolKind::Parameter);
    }

    #[test]
    fn extract_local_variable_in_function() {
        let syms = parse_and_extract("int main() { int x = 1; int y; return x + y; }");
        let x = syms.iter().find(|s| s.name == "x");
        let y = syms.iter().find(|s| s.name == "y");
        assert!(x.is_some(), "local `x` missing");
        assert!(y.is_some(), "local `y` missing");
        assert_eq!(x.unwrap().kind, SymbolKind::Variable);
        assert_eq!(y.unwrap().kind, SymbolKind::Variable);
    }

    #[test]
    fn extract_local_pointer_variable() {
        let syms = parse_and_extract("void f() { int *ptr; int *q = 0; }");
        let ptr = syms.iter().find(|s| s.name == "ptr");
        let q = syms.iter().find(|s| s.name == "q");
        assert!(ptr.is_some(), "pointer local `ptr` missing");
        assert!(q.is_some(), "initialized pointer local `q` missing");
        assert_eq!(ptr.unwrap().kind, SymbolKind::Variable);
        assert_eq!(q.unwrap().kind, SymbolKind::Variable);
    }
}
