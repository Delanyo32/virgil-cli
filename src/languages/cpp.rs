use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──
// C++ extends C with classes, namespaces, and qualified identifiers

const CPP_SYMBOL_QUERY: &str = r#"
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

(class_specifier
  name: (type_identifier) @name
  body: (field_declaration_list)) @definition

(namespace_definition
  name: (_) @name) @definition

(type_definition
  declarator: (type_identifier) @name) @definition

(preproc_def
  name: (identifier) @name) @definition

(preproc_function_def
  name: (identifier) @name) @definition
"#;

// ── Import queries ── (same as C: #include directives)

const CPP_IMPORT_QUERY: &str = r#"
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
    let query = Query::new(&ts_lang, CPP_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, CPP_IMPORT_QUERY)
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

        let kind = determine_cpp_kind(def_node);
        let Some(kind) = kind else { continue };

        let is_exported = is_exported_cpp(def_node, source);

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

fn determine_cpp_kind(def_node: tree_sitter::Node) -> Option<SymbolKind> {
    match def_node.kind() {
        "class_specifier" => Some(SymbolKind::Class),
        "namespace_definition" => Some(SymbolKind::Namespace),
        "function_definition" => Some(SymbolKind::Function),
        "declaration" => {
            let mut cursor = def_node.walk();
            for child in def_node.children(&mut cursor) {
                if child.kind() == "function_declarator" {
                    return Some(SymbolKind::Function);
                }
            }
            Some(SymbolKind::Variable)
        }
        "struct_specifier" => Some(SymbolKind::Struct),
        "union_specifier" => Some(SymbolKind::Union),
        "enum_specifier" => Some(SymbolKind::Enum),
        "type_definition" => Some(SymbolKind::Typedef),
        "preproc_def" | "preproc_function_def" => Some(SymbolKind::Macro),
        _ => None,
    }
}

fn is_exported_cpp(def_node: tree_sitter::Node, source: &[u8]) -> bool {
    // Macros, types, and namespaces are always "exported"
    match def_node.kind() {
        "preproc_def" | "preproc_function_def" | "struct_specifier" | "union_specifier"
        | "enum_specifier" | "type_definition" | "class_specifier" | "namespace_definition" => {
            return true
        }
        _ => {}
    }

    // Check for `static` storage class
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "storage_class_specifier"
            && child.utf8_text(source).unwrap_or("") == "static"
        {
            return false;
        }
    }

    true
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
            line: include_node.start_position().row as u32,
            is_external: is_system,
        });
    }

    imports
}

fn strip_include_path(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('<') && s.ends_with('>') {
        s[1..s.len() - 1].to_string()
    } else if s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
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
        "class_specifier" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("class".to_string()))
        }
        "struct_specifier" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("struct".to_string()))
        }
        "namespace_definition" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("namespace".to_string()))
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
            let name = node
                .child_by_field_name("declarator")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
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

fn find_identifier_recursive(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "qualified_identifier" => {
            return node.utf8_text(source).ok().map(|s| s.to_string());
        }
        _ => {}
    }
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

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::create_parser;

    fn parse_and_extract(source: &str) -> Vec<SymbolInfo> {
        let mut parser = create_parser(Language::Cpp).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Cpp).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.cpp")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::Cpp).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::Cpp).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.cpp")
    }

    #[test]
    fn extract_class() {
        let syms = parse_and_extract("class Foo { };");
        let s = syms.iter().find(|s| s.name == "Foo");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Class);
    }

    #[test]
    fn extract_namespace() {
        let syms = parse_and_extract("namespace MyApp { }");
        let s = syms.iter().find(|s| s.name == "MyApp");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Namespace);
    }

    #[test]
    fn extract_function() {
        let syms = parse_and_extract("int main() { return 0; }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "main");
        assert_eq!(syms[0].kind, SymbolKind::Function);
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
    fn static_function_not_exported() {
        let syms = parse_and_extract("static void helper() { }");
        assert_eq!(syms.len(), 1);
        assert!(!syms[0].is_exported);
    }

    #[test]
    fn include_directive() {
        let imports = parse_and_extract_imports("#include <iostream>\n#include \"myclass.h\"");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module_specifier, "iostream");
        assert!(imports[0].is_external);
        assert_eq!(imports[1].module_specifier, "myclass.h");
        assert!(!imports[1].is_external);
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("");
        assert!(syms.is_empty());
    }
}
