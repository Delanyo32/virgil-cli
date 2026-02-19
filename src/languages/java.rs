use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──

const JAVA_SYMBOL_QUERY: &str = r#"
(class_declaration
  name: (identifier) @name) @definition

(interface_declaration
  name: (identifier) @name) @definition

(enum_declaration
  name: (identifier) @name) @definition

(record_declaration
  name: (identifier) @name) @definition

(annotation_type_declaration
  name: (identifier) @name) @definition

(method_declaration
  name: (identifier) @name) @definition

(constructor_declaration
  name: (identifier) @name) @definition

(field_declaration
  declarator: (variable_declarator
    name: (identifier) @name)) @definition
"#;

// ── Import queries ──

const JAVA_IMPORT_QUERY: &str = r#"
(import_declaration) @import
"#;

// ── Comment queries ──

const JAVA_COMMENT_QUERY: &str = r#"
[
  (line_comment) @comment
  (block_comment) @comment
]
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, JAVA_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, JAVA_IMPORT_QUERY)
        .with_context(|| format!("failed to compile import query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, JAVA_COMMENT_QUERY)
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

        let kind = determine_java_kind(def_node);
        let Some(kind) = kind else { continue };

        let is_exported = is_exported_java(def_node, source);

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

fn determine_java_kind(def_node: tree_sitter::Node) -> Option<SymbolKind> {
    match def_node.kind() {
        "class_declaration" | "record_declaration" => Some(SymbolKind::Class),
        "interface_declaration" | "annotation_type_declaration" => Some(SymbolKind::Interface),
        "enum_declaration" => Some(SymbolKind::Enum),
        "method_declaration" | "constructor_declaration" => Some(SymbolKind::Method),
        "field_declaration" => Some(SymbolKind::Variable),
        _ => None,
    }
}

fn is_exported_java(def_node: tree_sitter::Node, source: &[u8]) -> bool {
    // Java wraps modifiers in a `modifiers` node
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mod_cursor = child.walk();
            for modifier in child.children(&mut mod_cursor) {
                let text = modifier.utf8_text(source).unwrap_or("");
                match text {
                    "public" => return true,
                    "private" | "protected" => return false,
                    _ => {}
                }
            }
        }
    }
    false // conservative default: package-private = not exported
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

        let (module_specifier, imported_name, is_static) = parse_java_import(&text);
        if module_specifier.is_empty() {
            continue;
        }

        let kind = if is_static {
            "static".to_string()
        } else {
            "import".to_string()
        };

        imports.push(ImportInfo {
            source_file: file_path.to_string(),
            module_specifier,
            imported_name: imported_name.clone(),
            local_name: imported_name,
            kind,
            is_type_only: false,
            line: node.start_position().row as u32,
            is_external: true, // Java imports are always external (no relative imports)
        });
    }

    imports
}

fn parse_java_import(text: &str) -> (String, String, bool) {
    let text = text.trim();
    let text = text.strip_prefix("import").unwrap_or(text).trim();
    let is_static = text.starts_with("static");
    let text = if is_static {
        text.strip_prefix("static").unwrap_or(text).trim()
    } else {
        text
    };
    let text = text.strip_suffix(';').unwrap_or(text).trim();

    if text.is_empty() {
        return (String::new(), String::new(), is_static);
    }

    let module_specifier = text.to_string();

    // Handle wildcards: import java.util.* → imported_name = "*"
    let imported_name = if text.ends_with(".*") {
        "*".to_string()
    } else {
        text.rsplit('.').next().unwrap_or(text).to_string()
    };

    (module_specifier, imported_name, is_static)
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
        "interface_declaration" | "annotation_type_declaration" => {
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
        "field_declaration" => {
            let name = extract_field_name(node, source);
            (name, Some("variable".to_string()))
        }
        _ => (None, None),
    }
}

fn extract_field_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Drill through variable_declarator → name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            return child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
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
        let mut parser = create_parser(Language::Java).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Java).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "Test.java")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::Java).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::Java).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "Test.java")
    }

    fn parse_and_extract_comments(source: &str) -> Vec<CommentInfo> {
        let mut parser = create_parser(Language::Java).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_comment_query(Language::Java).expect("compile comment query");
        extract_comments(&tree, source.as_bytes(), &query, "Test.java")
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
    fn extract_package_private_class() {
        let syms = parse_and_extract("class Foo { }");
        let s = syms.iter().find(|s| s.name == "Foo");
        assert!(s.is_some());
        assert!(!s.unwrap().is_exported); // package-private = not exported
    }

    #[test]
    fn extract_interface() {
        let syms = parse_and_extract("public interface Foo { }");
        let s = syms.iter().find(|s| s.name == "Foo");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Interface);
        assert!(s.unwrap().is_exported);
    }

    #[test]
    fn extract_enum() {
        let syms = parse_and_extract("public enum Color { RED, GREEN, BLUE }");
        let s = syms.iter().find(|s| s.name == "Color");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn extract_method() {
        let syms = parse_and_extract("public class Foo { public void bar() { } }");
        let method = syms.iter().find(|s| s.name == "bar");
        assert!(method.is_some());
        assert_eq!(method.unwrap().kind, SymbolKind::Method);
        assert!(method.unwrap().is_exported);
    }

    #[test]
    fn extract_constructor() {
        let syms = parse_and_extract("public class Foo { public Foo() { } }");
        let ctor = syms.iter().find(|s| s.name == "Foo" && s.kind == SymbolKind::Method);
        assert!(ctor.is_some());
    }

    #[test]
    fn extract_field() {
        let syms = parse_and_extract("public class Foo { private int count; }");
        let f = syms.iter().find(|s| s.name == "count");
        assert!(f.is_some());
        assert_eq!(f.unwrap().kind, SymbolKind::Variable);
        assert!(!f.unwrap().is_exported);
    }

    #[test]
    fn extract_record() {
        let syms = parse_and_extract("public record Point(int x, int y) { }");
        let s = syms.iter().find(|s| s.name == "Point");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Class);
    }

    #[test]
    fn extract_annotation_type() {
        let syms = parse_and_extract("public @interface MyAnnotation { }");
        let s = syms.iter().find(|s| s.name == "MyAnnotation");
        assert!(s.is_some());
        assert_eq!(s.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn simple_import() {
        let imports = parse_and_extract_imports("import java.util.List;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "java.util.List");
        assert_eq!(imports[0].imported_name, "List");
        assert_eq!(imports[0].kind, "import");
        assert!(imports[0].is_external);
    }

    #[test]
    fn wildcard_import() {
        let imports = parse_and_extract_imports("import java.util.*;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "java.util.*");
        assert_eq!(imports[0].imported_name, "*");
    }

    #[test]
    fn static_import() {
        let imports = parse_and_extract_imports("import static java.lang.Math.PI;");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "java.lang.Math.PI");
        assert_eq!(imports[0].imported_name, "PI");
        assert_eq!(imports[0].kind, "static");
    }

    #[test]
    fn line_comment() {
        let comments = parse_and_extract_comments("// a line comment\nclass Foo {}");
        let c = comments.iter().find(|c| c.text.contains("a line comment"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, "line");
    }

    #[test]
    fn block_comment() {
        let comments = parse_and_extract_comments("/* block comment */\nclass Foo {}");
        let c = comments.iter().find(|c| c.text.contains("block comment"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, "block");
    }

    #[test]
    fn doc_comment() {
        let comments = parse_and_extract_comments("/** Javadoc */\npublic class Foo {}");
        let c = comments.iter().find(|c| c.text.contains("Javadoc"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().kind, "doc");
    }

    #[test]
    fn comment_associated_symbol() {
        let comments =
            parse_and_extract_comments("/** Describes Foo */\npublic class Foo {}");
        let c = comments.iter().find(|c| c.text.contains("Describes Foo"));
        assert!(c.is_some());
        assert_eq!(c.unwrap().associated_symbol.as_deref(), Some("Foo"));
        assert_eq!(c.unwrap().associated_symbol_kind.as_deref(), Some("class"));
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("");
        assert!(syms.is_empty());
    }
}
