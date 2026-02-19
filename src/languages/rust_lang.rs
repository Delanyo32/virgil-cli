use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──

const RUST_SYMBOL_QUERY: &str = r#"
(function_item
  name: (identifier) @name) @definition

(struct_item
  name: (type_identifier) @name) @definition

(enum_item
  name: (type_identifier) @name) @definition

(trait_item
  name: (type_identifier) @name) @definition

(type_item
  name: (type_identifier) @name) @definition

(const_item
  name: (identifier) @name) @definition

(static_item
  name: (identifier) @name) @definition

(union_item
  name: (type_identifier) @name) @definition

(mod_item
  name: (identifier) @name) @definition

(macro_definition
  name: (identifier) @name) @definition
"#;

// ── Import queries ──

const RUST_IMPORT_QUERY: &str = r#"
(use_declaration
  argument: (_) @path) @import
"#;

// ── Comment queries ──

const RUST_COMMENT_QUERY: &str = r#"
[
  (line_comment) @comment
  (block_comment) @comment
]
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, RUST_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, RUST_IMPORT_QUERY)
        .with_context(|| format!("failed to compile import query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, RUST_COMMENT_QUERY)
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

        let kind = determine_rust_kind(def_node);
        let Some(kind) = kind else { continue };

        let is_exported = is_exported_rust(def_node);

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

fn determine_rust_kind(def_node: tree_sitter::Node) -> Option<SymbolKind> {
    match def_node.kind() {
        "function_item" => {
            // Check if inside impl_item or trait_item (method vs function)
            if is_inside_impl_or_trait(def_node) {
                Some(SymbolKind::Method)
            } else {
                Some(SymbolKind::Function)
            }
        }
        "struct_item" => Some(SymbolKind::Struct),
        "enum_item" => Some(SymbolKind::Enum),
        "trait_item" => Some(SymbolKind::Trait),
        "type_item" => Some(SymbolKind::TypeAlias),
        "const_item" => Some(SymbolKind::Constant),
        "static_item" => Some(SymbolKind::Variable),
        "union_item" => Some(SymbolKind::Union),
        "mod_item" => Some(SymbolKind::Module),
        "macro_definition" => Some(SymbolKind::Macro),
        _ => None,
    }
}

fn is_inside_impl_or_trait(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "impl_item" | "trait_item" => return true,
            "declaration_list" => {
                // declaration_list is the body of impl/trait, keep going up
                current = parent.parent();
                continue;
            }
            _ => return false,
        }
    }
    false
}

fn is_exported_rust(def_node: tree_sitter::Node) -> bool {
    // Check for visibility_modifier child node
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return true; // Any pub variant means exported
        }
    }
    false
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
    let import_idx = query.capture_index_for_name("import");

    let mut imports = Vec::new();

    while let Some(m) = matches.next() {
        let path_cap = path_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let import_cap = import_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));

        let (Some(path_cap), Some(import_cap)) = (path_cap, import_cap) else {
            continue;
        };

        let path_node = path_cap.node;
        let import_node = import_cap.node;
        let line = import_node.start_position().row as u32;

        let path_text = path_node.utf8_text(source).unwrap_or("").to_string();
        if path_text.is_empty() {
            continue;
        }

        // Extract individual imports from the use path
        extract_use_imports(&path_text, file_path, line, &mut imports);
    }

    imports
}

fn extract_use_imports(
    path_text: &str,
    file_path: &str,
    line: u32,
    imports: &mut Vec<ImportInfo>,
) {
    let is_internal = path_text.starts_with("crate::")
        || path_text.starts_with("self::")
        || path_text.starts_with("super::");

    // Handle grouped imports: use std::collections::{HashMap, HashSet}
    if let Some(brace_start) = path_text.find('{') {
        let prefix = &path_text[..brace_start];
        let brace_end = path_text.rfind('}').unwrap_or(path_text.len());
        let inner = &path_text[brace_start + 1..brace_end];

        for item in inner.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }

            let (imported_name, local_name) = if let Some((name, alias)) = item.split_once(" as ")
            {
                (name.trim().to_string(), alias.trim().to_string())
            } else {
                let name = item.split("::").last().unwrap_or(item).trim().to_string();
                (name.clone(), name)
            };

            let module = format!("{}{}", prefix, item).trim().to_string();

            imports.push(ImportInfo {
                source_file: file_path.to_string(),
                module_specifier: module,
                imported_name,
                local_name,
                kind: "use".to_string(),
                is_type_only: false,
                line,
                is_external: !is_internal,
            });
        }
    } else {
        // Simple path: use std::collections::HashMap or use std::collections::HashMap as Map
        let (module, imported_name, local_name) =
            if let Some((path, alias)) = path_text.split_once(" as ") {
                let name = path.split("::").last().unwrap_or(path).trim().to_string();
                (
                    path.trim().to_string(),
                    name,
                    alias.trim().to_string(),
                )
            } else if path_text.ends_with("::*") {
                (
                    path_text.to_string(),
                    "*".to_string(),
                    "*".to_string(),
                )
            } else {
                let name = path_text
                    .split("::")
                    .last()
                    .unwrap_or(path_text)
                    .trim()
                    .to_string();
                (path_text.to_string(), name.clone(), name)
            };

        imports.push(ImportInfo {
            source_file: file_path.to_string(),
            module_specifier: module,
            imported_name,
            local_name,
            kind: "use".to_string(),
            is_type_only: false,
            line,
            is_external: !is_internal,
        });
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
    if trimmed.starts_with("///") || trimmed.starts_with("//!") {
        "doc".to_string()
    } else if trimmed.starts_with("/**") || trimmed.starts_with("/*!") {
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
        "function_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            let kind = if is_inside_impl_or_trait(node) {
                "method"
            } else {
                "function"
            };
            (name, Some(kind.to_string()))
        }
        "struct_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("struct".to_string()))
        }
        "enum_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("enum".to_string()))
        }
        "trait_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("trait".to_string()))
        }
        "type_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("type_alias".to_string()))
        }
        "const_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("constant".to_string()))
        }
        "static_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("variable".to_string()))
        }
        "union_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("union".to_string()))
        }
        "mod_item" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("module".to_string()))
        }
        "macro_definition" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("macro".to_string()))
        }
        _ => (None, None),
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::create_parser;

    fn parse_and_extract(source: &str) -> Vec<SymbolInfo> {
        let mut parser = create_parser(Language::Rust).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Rust).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.rs")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::Rust).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::Rust).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.rs")
    }

    fn parse_and_extract_comments(source: &str) -> Vec<CommentInfo> {
        let mut parser = create_parser(Language::Rust).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_comment_query(Language::Rust).expect("compile comment query");
        extract_comments(&tree, source.as_bytes(), &query, "test.rs")
    }

    #[test]
    fn extract_function() {
        let syms = parse_and_extract("fn main() {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "main");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(!syms[0].is_exported);
    }

    #[test]
    fn extract_pub_function() {
        let syms = parse_and_extract("pub fn hello() {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "hello");
        assert!(syms[0].is_exported);
    }

    #[test]
    fn extract_struct() {
        let syms = parse_and_extract("pub struct Point { x: i32, y: i32 }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Point");
        assert_eq!(syms[0].kind, SymbolKind::Struct);
        assert!(syms[0].is_exported);
    }

    #[test]
    fn extract_enum() {
        let syms = parse_and_extract("enum Color { Red, Green, Blue }");
        let s = syms.iter().find(|s| s.name == "Color");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn extract_trait() {
        let syms = parse_and_extract("pub trait Display { fn fmt(&self); }");
        let t = syms.iter().find(|s| s.name == "Display");
        assert!(t.is_some());
        assert_eq!(t.unwrap().kind, SymbolKind::Trait);
        assert!(t.unwrap().is_exported);
    }

    #[test]
    fn extract_method_in_impl() {
        let syms = parse_and_extract("struct Foo {}\nimpl Foo { fn bar(&self) {} }");
        let method = syms.iter().find(|s| s.name == "bar");
        assert!(method.is_some());
        assert_eq!(method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn extract_const() {
        let syms = parse_and_extract("const MAX: u32 = 100;");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MAX");
        assert_eq!(syms[0].kind, SymbolKind::Constant);
    }

    #[test]
    fn extract_static() {
        let syms = parse_and_extract("static COUNT: u32 = 0;");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "COUNT");
        assert_eq!(syms[0].kind, SymbolKind::Variable);
    }

    #[test]
    fn extract_type_alias() {
        let syms = parse_and_extract("type Result<T> = std::result::Result<T, Error>;");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Result");
        assert_eq!(syms[0].kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn extract_module() {
        let syms = parse_and_extract("mod utils {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "utils");
        assert_eq!(syms[0].kind, SymbolKind::Module);
    }

    #[test]
    fn extract_macro() {
        let syms = parse_and_extract("macro_rules! my_macro { () => {} }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "my_macro");
        assert_eq!(syms[0].kind, SymbolKind::Macro);
    }

    #[test]
    fn extract_union() {
        let syms = parse_and_extract("union MyUnion { i: i32, f: f32 }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyUnion");
        assert_eq!(syms[0].kind, SymbolKind::Union);
    }

    #[test]
    fn simple_use_import() {
        let imports = parse_and_extract_imports("use std::collections::HashMap;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "std::collections::HashMap");
        assert_eq!(imports[0].imported_name, "HashMap");
        assert_eq!(imports[0].kind, "use");
        assert!(imports[0].is_external);
    }

    #[test]
    fn crate_internal_import() {
        let imports = parse_and_extract_imports("use crate::models::SymbolInfo;");
        assert_eq!(imports.len(), 1);
        assert!(!imports[0].is_external);
    }

    #[test]
    fn self_import() {
        let imports = parse_and_extract_imports("use self::utils::helper;");
        assert_eq!(imports.len(), 1);
        assert!(!imports[0].is_external);
    }

    #[test]
    fn super_import() {
        let imports = parse_and_extract_imports("use super::models::SymbolKind;");
        assert_eq!(imports.len(), 1);
        assert!(!imports[0].is_external);
    }

    #[test]
    fn wildcard_import() {
        let imports = parse_and_extract_imports("use std::io::*;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "*");
    }

    #[test]
    fn aliased_import() {
        let imports = parse_and_extract_imports("use std::collections::HashMap as Map;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "HashMap");
        assert_eq!(imports[0].local_name, "Map");
    }

    #[test]
    fn doc_comment() {
        let comments = parse_and_extract_comments("/// This is a doc comment\nfn foo() {}");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, "doc");
        assert_eq!(comments[0].associated_symbol.as_deref(), Some("foo"));
    }

    #[test]
    fn inner_doc_comment() {
        let comments = parse_and_extract_comments("//! Module doc");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, "doc");
    }

    #[test]
    fn line_comment() {
        let comments = parse_and_extract_comments("// Just a comment");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, "line");
    }

    #[test]
    fn block_comment() {
        let comments = parse_and_extract_comments("/* block */");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, "block");
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("");
        assert!(syms.is_empty());
    }
}
