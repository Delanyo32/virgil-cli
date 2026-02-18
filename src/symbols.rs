use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{SymbolInfo, SymbolKind};

const TS_QUERY: &str = r#"
(function_declaration
  name: (identifier) @name) @definition

(class_declaration
  name: (type_identifier) @name) @definition

(method_definition
  name: (property_identifier) @name) @definition

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: (_) @value)) @definition

(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: (_) @value)) @definition

(interface_declaration
  name: (type_identifier) @name) @definition

(type_alias_declaration
  name: (type_identifier) @name) @definition

(enum_declaration
  name: (identifier) @name) @definition
"#;

const JS_QUERY: &str = r#"
(function_declaration
  name: (identifier) @name) @definition

(class_declaration
  name: (identifier) @name) @definition

(method_definition
  name: (property_identifier) @name) @definition

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: (_) @value)) @definition

(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: (_) @value)) @definition
"#;

pub fn compile_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let source = match language {
        Language::JavaScript => JS_QUERY,
        _ => TS_QUERY,
    };
    let query = Query::new(&ts_lang, source)
        .with_context(|| format!("failed to compile query for {language}"))?;
    Ok(Arc::new(query))
}

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
    let value_idx = query.capture_index_for_name("value");

    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        let name_cap = name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let def_cap = definition_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let value_cap = value_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));

        let (Some(name_cap), Some(def_cap)) = (name_cap, def_cap) else {
            continue;
        };

        let name_node = name_cap.node;
        let def_node = def_cap.node;

        let name = name_node.utf8_text(source).unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }

        let kind = determine_kind(def_node.kind(), value_cap.map(|c| c.node.kind()));
        let Some(kind) = kind else { continue };

        // Check if parent is an export_statement
        let is_exported = def_node
            .parent()
            .is_some_and(|p| p.kind() == "export_statement");

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

fn determine_kind(def_kind: &str, value_kind: Option<&str>) -> Option<SymbolKind> {
    match def_kind {
        "function_declaration" => Some(SymbolKind::Function),
        "class_declaration" => Some(SymbolKind::Class),
        "method_definition" => Some(SymbolKind::Method),
        "interface_declaration" => Some(SymbolKind::Interface),
        "type_alias_declaration" => Some(SymbolKind::TypeAlias),
        "enum_declaration" => Some(SymbolKind::Enum),
        "lexical_declaration" | "variable_declaration" => {
            if let Some(vk) = value_kind {
                if vk == "arrow_function" {
                    Some(SymbolKind::ArrowFunction)
                } else {
                    Some(SymbolKind::Variable)
                }
            } else {
                Some(SymbolKind::Variable)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::create_parser;

    // Helper: parse source in-memory and extract symbols
    fn parse_and_extract(source: &str, language: Language) -> Vec<SymbolInfo> {
        let mut parser = create_parser(language).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_query(language).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.ts")
    }

    #[test]
    fn determine_kind_function() {
        assert_eq!(determine_kind("function_declaration", None), Some(SymbolKind::Function));
    }

    #[test]
    fn determine_kind_class() {
        assert_eq!(determine_kind("class_declaration", None), Some(SymbolKind::Class));
    }

    #[test]
    fn determine_kind_method() {
        assert_eq!(determine_kind("method_definition", None), Some(SymbolKind::Method));
    }

    #[test]
    fn determine_kind_interface() {
        assert_eq!(determine_kind("interface_declaration", None), Some(SymbolKind::Interface));
    }

    #[test]
    fn determine_kind_type_alias() {
        assert_eq!(determine_kind("type_alias_declaration", None), Some(SymbolKind::TypeAlias));
    }

    #[test]
    fn determine_kind_enum() {
        assert_eq!(determine_kind("enum_declaration", None), Some(SymbolKind::Enum));
    }

    #[test]
    fn determine_kind_arrow_function() {
        assert_eq!(
            determine_kind("lexical_declaration", Some("arrow_function")),
            Some(SymbolKind::ArrowFunction)
        );
    }

    #[test]
    fn determine_kind_variable() {
        assert_eq!(
            determine_kind("lexical_declaration", Some("string")),
            Some(SymbolKind::Variable)
        );
    }

    #[test]
    fn determine_kind_variable_declaration() {
        assert_eq!(
            determine_kind("variable_declaration", Some("number")),
            Some(SymbolKind::Variable)
        );
    }

    #[test]
    fn determine_kind_variable_no_value() {
        assert_eq!(
            determine_kind("lexical_declaration", None),
            Some(SymbolKind::Variable)
        );
    }

    #[test]
    fn determine_kind_unknown() {
        assert_eq!(determine_kind("import_statement", None), None);
    }

    #[test]
    fn extract_exported_function() {
        let syms = parse_and_extract("export function greet() {}", Language::TypeScript);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(syms[0].is_exported);
    }

    #[test]
    fn extract_non_exported_function() {
        let syms = parse_and_extract("function helper() {}", Language::TypeScript);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "helper");
        assert!(!syms[0].is_exported);
    }

    #[test]
    fn extract_class_with_method() {
        let source = "class Foo { bar() {} }";
        let syms = parse_and_extract(source, Language::TypeScript);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"bar"));
        let foo = syms.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.kind, SymbolKind::Class);
        let bar = syms.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
    }

    #[test]
    fn extract_arrow_function_vs_variable() {
        let source = "const handler = () => {};\nconst PI = 3.14;";
        let syms = parse_and_extract(source, Language::TypeScript);
        let handler = syms.iter().find(|s| s.name == "handler").unwrap();
        assert_eq!(handler.kind, SymbolKind::ArrowFunction);
        let pi = syms.iter().find(|s| s.name == "PI").unwrap();
        assert_eq!(pi.kind, SymbolKind::Variable);
    }

    #[test]
    fn extract_interface_type_enum() {
        let source = r#"
            interface User { id: number; }
            type UserId = number;
            enum Role { Admin, User }
        "#;
        let syms = parse_and_extract(source, Language::TypeScript);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"User"));
        assert!(names.contains(&"UserId"));
        assert!(names.contains(&"Role"));
    }

    #[test]
    fn destructured_variables_skipped() {
        let source = "const { a, b } = { a: 1, b: 2 };";
        let syms = parse_and_extract(source, Language::TypeScript);
        // Destructured names are not identifiers, should be skipped
        assert!(syms.is_empty());
    }

    #[test]
    fn extract_js_symbols() {
        let source = "function add() {}\nclass Calc {}\nconst x = 1;\nconst f = () => {};";
        let syms = parse_and_extract(source, Language::JavaScript);
        assert_eq!(syms.len(), 4);
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("", Language::TypeScript);
        assert!(syms.is_empty());
    }

    #[test]
    fn positions_are_sane() {
        let source = "function foo() {\n  return 1;\n}";
        let syms = parse_and_extract(source, Language::TypeScript);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].start_line, 0); // tree-sitter is 0-indexed
        assert!(syms[0].end_line >= syms[0].start_line);
    }
}
