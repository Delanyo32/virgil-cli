use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──

const PHP_SYMBOL_QUERY: &str = r#"
(function_definition
  name: (name) @name) @definition

(class_declaration
  name: (name) @name) @definition

(interface_declaration
  name: (name) @name) @definition

(trait_declaration
  name: (name) @name) @definition

(enum_declaration
  name: (name) @name) @definition

(method_declaration
  name: (name) @name) @definition

(property_declaration) @definition

(const_declaration) @definition

(namespace_definition
  name: (namespace_name) @name) @definition
"#;

// ── Import queries ──

const PHP_IMPORT_QUERY: &str = r#"
(namespace_use_declaration) @import

(expression_statement
  (require_expression) @require)

(expression_statement
  (require_once_expression) @require)

(expression_statement
  (include_expression) @include)

(expression_statement
  (include_once_expression) @include)
"#;

// ── Comment queries ──

const PHP_COMMENT_QUERY: &str = r#"
(comment) @comment
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, PHP_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, PHP_IMPORT_QUERY)
        .with_context(|| format!("failed to compile import query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, PHP_COMMENT_QUERY)
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
        let def_cap = definition_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let Some(def_cap) = def_cap else {
            continue;
        };
        let def_node = def_cap.node;

        let kind = determine_php_kind(def_node);
        let Some(kind) = kind else { continue };

        // For property_declaration and const_declaration, extract name manually
        let name = if def_node.kind() == "property_declaration" {
            extract_property_name(def_node, source)
        } else if def_node.kind() == "const_declaration" {
            extract_const_name(def_node, source)
        } else {
            let name_cap = name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
            name_cap.and_then(|cap| {
                let text = cap.node.utf8_text(source).unwrap_or("");
                if text.is_empty() { None } else { Some(text.to_string()) }
            })
        };

        let Some(name) = name else { continue };

        let is_exported = is_exported_php(def_node, source);

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

fn determine_php_kind(def_node: tree_sitter::Node) -> Option<SymbolKind> {
    match def_node.kind() {
        "function_definition" => Some(SymbolKind::Function),
        "class_declaration" => Some(SymbolKind::Class),
        "interface_declaration" => Some(SymbolKind::Interface),
        "trait_declaration" => Some(SymbolKind::Trait),
        "enum_declaration" => Some(SymbolKind::Enum),
        "method_declaration" => Some(SymbolKind::Method),
        "property_declaration" => Some(SymbolKind::Property),
        "const_declaration" => Some(SymbolKind::Constant),
        "namespace_definition" => Some(SymbolKind::Namespace),
        _ => None,
    }
}

fn extract_property_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // property_declaration > property_element > variable_name > $name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "property_element" {
            let mut inner_cursor = child.walk();
            for inner_child in child.children(&mut inner_cursor) {
                if inner_child.kind() == "variable_name" {
                    let text = inner_child.utf8_text(source).unwrap_or("");
                    // Strip leading $
                    let name = text.strip_prefix('$').unwrap_or(text);
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}

fn extract_const_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // const_declaration > const_element > name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_element" {
            let mut inner_cursor = child.walk();
            for inner_child in child.children(&mut inner_cursor) {
                if inner_child.kind() == "name" {
                    let text = inner_child.utf8_text(source).unwrap_or("");
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
        }
    }
    None
}

fn is_exported_php(def_node: tree_sitter::Node, source: &[u8]) -> bool {
    match def_node.kind() {
        // Top-level definitions are always exported
        "function_definition" | "class_declaration" | "interface_declaration"
        | "trait_declaration" | "enum_declaration" | "namespace_definition" => true,
        // Methods, properties, constants: check visibility modifier
        "method_declaration" | "property_declaration" | "const_declaration" => {
            let mut cursor = def_node.walk();
            for child in def_node.children(&mut cursor) {
                if child.kind() == "visibility_modifier" {
                    let text = child.utf8_text(source).unwrap_or("");
                    return text == "public";
                }
            }
            true // PHP default visibility is public
        }
        _ => true,
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

    let import_idx = query.capture_index_for_name("import");
    let require_idx = query.capture_index_for_name("require");
    let include_idx = query.capture_index_for_name("include");

    let mut imports = Vec::new();

    while let Some(m) = matches.next() {
        // Handle namespace_use_declaration
        if let Some(import_cap) =
            import_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx))
        {
            let node = import_cap.node;
            let text = node.utf8_text(source).unwrap_or("").to_string();
            parse_use_declaration(&text, file_path, node.start_position().row as u32, &mut imports);
            continue;
        }

        // Handle require/require_once
        if let Some(require_cap) =
            require_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx))
        {
            let node = require_cap.node;
            let text = node.utf8_text(source).unwrap_or("").to_string();
            if let Some(path) = extract_string_arg(&text) {
                let is_external = !path.starts_with('.');
                imports.push(ImportInfo {
                    source_file: file_path.to_string(),
                    module_specifier: path.clone(),
                    imported_name: path.rsplit('/').next().unwrap_or(&path).to_string(),
                    local_name: "*".to_string(),
                    kind: "require".to_string(),
                    is_type_only: false,
                    line: node.start_position().row as u32,
                    is_external,
                });
            }
            continue;
        }

        // Handle include/include_once
        if let Some(include_cap) =
            include_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx))
        {
            let node = include_cap.node;
            let text = node.utf8_text(source).unwrap_or("").to_string();
            if let Some(path) = extract_string_arg(&text) {
                let is_external = !path.starts_with('.');
                imports.push(ImportInfo {
                    source_file: file_path.to_string(),
                    module_specifier: path.clone(),
                    imported_name: path.rsplit('/').next().unwrap_or(&path).to_string(),
                    local_name: "*".to_string(),
                    kind: "include".to_string(),
                    is_type_only: false,
                    line: node.start_position().row as u32,
                    is_external,
                });
            }
            continue;
        }
    }

    imports
}

fn parse_use_declaration(text: &str, file_path: &str, line: u32, imports: &mut Vec<ImportInfo>) {
    let text = text.trim();
    let text = text.strip_prefix("use").unwrap_or(text).trim();
    let text = text.strip_suffix(';').unwrap_or(text).trim();

    if text.is_empty() {
        return;
    }

    // Handle grouped use: App\Models\{User, Post}
    if let Some(brace_start) = text.find('{') {
        let prefix = text[..brace_start].trim().trim_end_matches('\\');
        let brace_end = text.rfind('}').unwrap_or(text.len());
        let inner = &text[brace_start + 1..brace_end];

        for item in inner.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }

            let (imported_name, local_name) = if let Some((name, alias)) = item.split_once(" as ")
            {
                let name = name.trim();
                let imported = name.rsplit('\\').next().unwrap_or(name);
                (imported.to_string(), alias.trim().to_string())
            } else {
                let imported = item.rsplit('\\').next().unwrap_or(item);
                (imported.to_string(), imported.to_string())
            };

            let module = format!("{}\\{}", prefix, item.split(" as ").next().unwrap_or(item).trim());

            imports.push(ImportInfo {
                source_file: file_path.to_string(),
                module_specifier: module,
                imported_name,
                local_name,
                kind: "use".to_string(),
                is_type_only: false,
                line,
                is_external: true,
            });
        }
    } else {
        // Simple use: App\Models\User or App\Models\User as U
        let (path, local_name) = if let Some((path, alias)) = text.split_once(" as ") {
            (path.trim(), alias.trim().to_string())
        } else {
            (text, text.rsplit('\\').next().unwrap_or(text).to_string())
        };

        let imported_name = path.rsplit('\\').next().unwrap_or(path).to_string();

        imports.push(ImportInfo {
            source_file: file_path.to_string(),
            module_specifier: path.to_string(),
            imported_name,
            local_name,
            kind: "use".to_string(),
            is_type_only: false,
            line,
            is_external: true,
        });
    }
}

fn extract_string_arg(text: &str) -> Option<String> {
    // Extract string from require('path') or require "path" or require_once 'path'
    // Find first quote character
    let single_start = text.find('\'');
    let double_start = text.find('"');

    let (start, quote) = match (single_start, double_start) {
        (Some(s), Some(d)) => {
            if s < d {
                (s, '\'')
            } else {
                (d, '"')
            }
        }
        (Some(s), None) => (s, '\''),
        (None, Some(d)) => (d, '"'),
        (None, None) => return None,
    };

    let rest = &text[start + 1..];
    let end = rest.find(quote)?;
    let path = &rest[..end];

    if path.is_empty() {
        return None;
    }

    Some(path.to_string())
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
    if trimmed.starts_with("/**") {
        "doc".to_string()
    } else if trimmed.starts_with("/*") {
        "block".to_string()
    } else {
        // // or # are both line comments
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
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("function".to_string()))
        }
        "class_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("class".to_string()))
        }
        "interface_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("interface".to_string()))
        }
        "trait_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("trait".to_string()))
        }
        "enum_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("enum".to_string()))
        }
        "method_declaration" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("method".to_string()))
        }
        "namespace_definition" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("namespace".to_string()))
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
        let mut parser = create_parser(Language::Php).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Php).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.php")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::Php).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::Php).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.php")
    }

    fn parse_and_extract_comments(source: &str) -> Vec<CommentInfo> {
        let mut parser = create_parser(Language::Php).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_comment_query(Language::Php).expect("compile comment query");
        extract_comments(&tree, source.as_bytes(), &query, "test.php")
    }

    #[test]
    fn extract_function() {
        let syms = parse_and_extract("<?php\nfunction hello() {}");
        let f = syms.iter().find(|s| s.name == "hello");
        assert!(f.is_some());
        assert_eq!(f.unwrap().kind, SymbolKind::Function);
        assert!(f.unwrap().is_exported);
    }

    #[test]
    fn extract_class() {
        let syms = parse_and_extract("<?php\nclass Foo {}");
        let s = syms.iter().find(|s| s.name == "Foo");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Class);
        assert!(s.unwrap().is_exported);
    }

    #[test]
    fn extract_interface() {
        let syms = parse_and_extract("<?php\ninterface Fooable {}");
        let s = syms.iter().find(|s| s.name == "Fooable");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn extract_trait() {
        let syms = parse_and_extract("<?php\ntrait Loggable {}");
        let s = syms.iter().find(|s| s.name == "Loggable");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Trait);
    }

    #[test]
    fn extract_enum() {
        let syms = parse_and_extract("<?php\nenum Color { case Red; case Green; }");
        let s = syms.iter().find(|s| s.name == "Color");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn extract_method() {
        let syms = parse_and_extract("<?php\nclass Foo { public function bar() {} }");
        let m = syms.iter().find(|s| s.name == "bar");
        assert!(m.is_some());
        assert_eq!(m.unwrap().kind, SymbolKind::Method);
        assert!(m.unwrap().is_exported);
    }

    #[test]
    fn extract_private_method() {
        let syms = parse_and_extract("<?php\nclass Foo { private function bar() {} }");
        let m = syms.iter().find(|s| s.name == "bar");
        assert!(m.is_some());
        assert!(!m.unwrap().is_exported);
    }

    #[test]
    fn extract_property() {
        let syms = parse_and_extract("<?php\nclass Foo { public $name = 'test'; }");
        let p = syms.iter().find(|s| s.name == "name");
        assert!(p.is_some());
        assert_eq!(p.unwrap().kind, SymbolKind::Property);
        assert!(p.unwrap().is_exported);
    }

    #[test]
    fn extract_const() {
        let syms = parse_and_extract("<?php\nclass Foo { const MAX = 100; }");
        let c = syms.iter().find(|s| s.name == "MAX");
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, SymbolKind::Constant);
    }

    #[test]
    fn extract_namespace() {
        let syms = parse_and_extract("<?php\nnamespace App\\Models;");
        let n = syms.iter().find(|s| s.name == "App\\Models");
        assert!(n.is_some());
        assert_eq!(n.unwrap().kind, SymbolKind::Namespace);
        assert!(n.unwrap().is_exported);
    }

    #[test]
    fn use_statement() {
        let imports = parse_and_extract_imports("<?php\nuse App\\Models\\User;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "App\\Models\\User");
        assert_eq!(imports[0].imported_name, "User");
        assert_eq!(imports[0].kind, "use");
        assert!(imports[0].is_external);
    }

    #[test]
    fn use_with_alias() {
        let imports = parse_and_extract_imports("<?php\nuse App\\Models\\User as U;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "User");
        assert_eq!(imports[0].local_name, "U");
    }

    #[test]
    fn grouped_use() {
        let imports =
            parse_and_extract_imports("<?php\nuse App\\Models\\{User, Post};");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].imported_name, "User");
        assert_eq!(imports[1].imported_name, "Post");
    }

    #[test]
    fn require_relative() {
        let imports = parse_and_extract_imports("<?php\nrequire './helpers.php';");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "./helpers.php");
        assert_eq!(imports[0].kind, "require");
        assert!(!imports[0].is_external);
    }

    #[test]
    fn require_absolute() {
        let imports = parse_and_extract_imports("<?php\nrequire 'vendor/autoload.php';");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, "require");
        assert!(imports[0].is_external);
    }

    #[test]
    fn line_comment() {
        let comments = parse_and_extract_comments("<?php\n// a line comment\nfunction foo() {}");
        let c = comments.iter().find(|c| c.text.contains("a line comment"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, "line");
    }

    #[test]
    fn hash_comment() {
        let comments = parse_and_extract_comments("<?php\n# a hash comment\nfunction foo() {}");
        let c = comments.iter().find(|c| c.text.contains("hash comment"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, "line");
    }

    #[test]
    fn block_comment() {
        let comments = parse_and_extract_comments("<?php\n/* block */\nfunction foo() {}");
        let c = comments.iter().find(|c| c.text.contains("block"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, "block");
    }

    #[test]
    fn doc_comment() {
        let comments = parse_and_extract_comments("<?php\n/** PHPDoc */\nfunction foo() {}");
        let c = comments.iter().find(|c| c.text.contains("PHPDoc"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, "doc");
    }

    #[test]
    fn comment_associated_symbol() {
        let comments =
            parse_and_extract_comments("<?php\n/** Describes Foo */\nclass Foo {}");
        let c = comments.iter().find(|c| c.text.contains("Describes Foo"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().associated_symbol.as_deref(), Some("Foo"));
        assert_eq!(c.unwrap().associated_symbol_kind.as_deref(), Some("class"));
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("<?php");
        assert!(syms.is_empty());
    }
}
