use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──

const GO_SYMBOL_QUERY: &str = r#"
(function_declaration
  name: (identifier) @name) @definition

(method_declaration
  name: (field_identifier) @name) @definition

(type_declaration
  (type_spec
    name: (type_identifier) @name) @definition)

(const_declaration
  (const_spec
    name: (identifier) @name) @definition)

(var_declaration
  (var_spec
    name: (identifier) @name) @definition)
"#;

// ── Import queries ──

const GO_IMPORT_QUERY: &str = r#"
(import_declaration
  (import_spec
    path: (interpreted_string_literal) @path) @import)

(import_declaration
  (import_spec_list
    (import_spec
      path: (interpreted_string_literal) @path) @import))
"#;

// ── Comment queries ──

const GO_COMMENT_QUERY: &str = r#"
(comment) @comment
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, GO_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, GO_IMPORT_QUERY)
        .with_context(|| format!("failed to compile import query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, GO_COMMENT_QUERY)
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

        let kind = determine_go_kind(def_node, source);
        let Some(kind) = kind else { continue };

        let is_exported = name.chars().next().is_some_and(|c| c.is_uppercase());

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

fn determine_go_kind(def_node: tree_sitter::Node, source: &[u8]) -> Option<SymbolKind> {
    match def_node.kind() {
        "function_declaration" => Some(SymbolKind::Function),
        "method_declaration" => Some(SymbolKind::Method),
        "type_spec" => {
            // Check what type it wraps
            let type_child = def_node.child_by_field_name("type");
            match type_child.map(|n| n.kind()) {
                Some("struct_type") => Some(SymbolKind::Struct),
                Some("interface_type") => Some(SymbolKind::Interface),
                _ => Some(SymbolKind::TypeAlias),
            }
        }
        "const_spec" => Some(SymbolKind::Constant),
        "var_spec" => Some(SymbolKind::Variable),
        _ => {
            // For parent nodes like type_declaration, const_declaration, var_declaration
            // walk children to find spec nodes
            let _ = source;
            None
        }
    }
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

        let raw_path = path_node.utf8_text(source).unwrap_or("").to_string();
        // Strip quotes
        let module_specifier = raw_path
            .trim_matches('"')
            .to_string();
        if module_specifier.is_empty() {
            continue;
        }

        // Get the last segment as the imported name
        let imported_name = module_specifier
            .rsplit('/')
            .next()
            .unwrap_or(&module_specifier)
            .to_string();

        // Check for alias (name field on import_spec)
        let local_name = import_node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| imported_name.clone());

        imports.push(ImportInfo {
            source_file: file_path.to_string(),
            module_specifier,
            imported_name,
            local_name,
            kind: "import".to_string(),
            is_type_only: false,
            line: import_node.start_position().row as u32,
            is_external: true, // Go has no syntactic internal/external distinction
        });
    }

    imports
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
    if trimmed.starts_with("/*") {
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
        "function_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("function".to_string()))
        }
        "method_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("method".to_string()))
        }
        "type_declaration" => {
            // Drill into type_spec
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_spec" {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string());
                    let type_child = child.child_by_field_name("type");
                    let kind_str = match type_child.map(|n| n.kind()) {
                        Some("struct_type") => "struct",
                        Some("interface_type") => "interface",
                        _ => "type_alias",
                    };
                    return (name, Some(kind_str.to_string()));
                }
            }
            (None, None)
        }
        "const_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "const_spec" {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string());
                    return (name, Some("constant".to_string()));
                }
            }
            (None, None)
        }
        "var_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "var_spec" {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string());
                    return (name, Some("variable".to_string()));
                }
            }
            (None, None)
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
        let mut parser = create_parser(Language::Go).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Go).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.go")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::Go).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::Go).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.go")
    }

    fn parse_and_extract_comments(source: &str) -> Vec<CommentInfo> {
        let mut parser = create_parser(Language::Go).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_comment_query(Language::Go).expect("compile comment query");
        extract_comments(&tree, source.as_bytes(), &query, "test.go")
    }

    #[test]
    fn extract_function() {
        let syms = parse_and_extract("package main\nfunc main() {}");
        let f = syms.iter().find(|s| s.name == "main");
        assert!(f.is_some());
        assert_eq!(f.unwrap().kind, SymbolKind::Function);
        assert!(!f.unwrap().is_exported); // lowercase
    }

    #[test]
    fn extract_exported_function() {
        let syms = parse_and_extract("package main\nfunc Hello() {}");
        let f = syms.iter().find(|s| s.name == "Hello");
        assert!(f.is_some());
        assert!(f.unwrap().is_exported); // uppercase
    }

    #[test]
    fn extract_method() {
        let syms = parse_and_extract("package main\ntype Foo struct{}\nfunc (f Foo) Bar() {}");
        let m = syms.iter().find(|s| s.name == "Bar");
        assert!(m.is_some());
        assert_eq!(m.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn extract_struct() {
        let syms = parse_and_extract("package main\ntype Point struct { X int; Y int }");
        let s = syms.iter().find(|s| s.name == "Point");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Struct);
        assert!(s.unwrap().is_exported);
    }

    #[test]
    fn extract_interface() {
        let syms = parse_and_extract("package main\ntype Reader interface { Read() }");
        let s = syms.iter().find(|s| s.name == "Reader");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn extract_const() {
        let syms = parse_and_extract("package main\nconst MaxSize = 100");
        let s = syms.iter().find(|s| s.name == "MaxSize");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Constant);
        assert!(s.unwrap().is_exported);
    }

    #[test]
    fn extract_var() {
        let syms = parse_and_extract("package main\nvar count int = 0");
        let s = syms.iter().find(|s| s.name == "count");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Variable);
        assert!(!s.unwrap().is_exported); // lowercase
    }

    #[test]
    fn single_import() {
        let imports = parse_and_extract_imports("package main\nimport \"fmt\"");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "fmt");
        assert_eq!(imports[0].imported_name, "fmt");
        assert_eq!(imports[0].kind, "import");
        assert!(imports[0].is_external);
    }

    #[test]
    fn grouped_imports() {
        let imports =
            parse_and_extract_imports("package main\nimport (\n\t\"fmt\"\n\t\"os\"\n)");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module_specifier, "fmt");
        assert_eq!(imports[1].module_specifier, "os");
    }

    #[test]
    fn import_with_path() {
        let imports =
            parse_and_extract_imports("package main\nimport \"net/http\"");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "net/http");
        assert_eq!(imports[0].imported_name, "http");
    }

    #[test]
    fn line_comment() {
        let comments = parse_and_extract_comments("package main\n// a comment");
        assert!(comments.iter().any(|c| c.kind == "line" && c.text.contains("a comment")));
    }

    #[test]
    fn block_comment() {
        let comments = parse_and_extract_comments("package main\n/* block */");
        assert!(comments.iter().any(|c| c.kind == "block"));
    }

    #[test]
    fn comment_associated_symbol() {
        let comments = parse_and_extract_comments("package main\n// Hello says hello\nfunc Hello() {}");
        let c = comments.iter().find(|c| c.text.contains("Hello says"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().associated_symbol.as_deref(), Some("Hello"));
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("package main");
        assert!(syms.is_empty());
    }
}
