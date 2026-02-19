use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──

const TS_SYMBOL_QUERY: &str = r#"
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

const JS_SYMBOL_QUERY: &str = r#"
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

// ── Import queries ──

const TS_IMPORT_QUERY: &str = r#"
(import_statement source: (string) @source) @import

(export_statement source: (string) @source) @reexport

(call_expression
  function: (import)
  arguments: (arguments (string) @source)) @dynamic_import

(call_expression
  function: (identifier) @fn_name
  arguments: (arguments (string) @source)) @call
"#;

const JS_IMPORT_QUERY: &str = r#"
(import_statement source: (string) @source) @import

(export_statement source: (string) @source) @reexport

(call_expression
  function: (import)
  arguments: (arguments (string) @source)) @dynamic_import

(call_expression
  function: (identifier) @fn_name
  arguments: (arguments (string) @source)) @call
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let source = match language {
        Language::JavaScript => JS_SYMBOL_QUERY,
        _ => TS_SYMBOL_QUERY,
    };
    let query = Query::new(&ts_lang, source)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let source = match language {
        Language::JavaScript => JS_IMPORT_QUERY,
        _ => TS_IMPORT_QUERY,
    };
    let query = Query::new(&ts_lang, source)
        .with_context(|| format!("failed to compile import query for {language}"))?;
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

// ── Import extraction ──

pub fn extract_imports(
    tree: &Tree,
    source: &[u8],
    query: &Query,
    file_path: &str,
) -> Vec<ImportInfo> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let source_idx = query.capture_index_for_name("source");
    let import_idx = query.capture_index_for_name("import");
    let reexport_idx = query.capture_index_for_name("reexport");
    let dynamic_import_idx = query.capture_index_for_name("dynamic_import");
    let call_idx = query.capture_index_for_name("call");
    let fn_name_idx = query.capture_index_for_name("fn_name");

    let mut imports = Vec::new();

    while let Some(m) = matches.next() {
        let source_cap = source_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let Some(source_cap) = source_cap else {
            continue;
        };

        let module_specifier = strip_quotes(source_cap.node.utf8_text(source).unwrap_or(""));
        if module_specifier.is_empty() {
            continue;
        }

        // Determine which pattern matched
        let has_import = import_idx
            .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
            .is_some();
        let has_reexport = reexport_idx
            .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
            .is_some();
        let has_dynamic = dynamic_import_idx
            .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
            .is_some();
        let has_call = call_idx
            .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
            .is_some();

        if has_import {
            let import_node = import_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                .unwrap()
                .node;
            let line = import_node.start_position().row as u32;
            let is_type_only = has_type_keyword(import_node);
            let extracted = extract_import_bindings(import_node, source);

            let is_external = ImportInfo::is_external_specifier(&module_specifier);

            if extracted.is_empty() {
                // Side-effect import: import "./polyfill"
                imports.push(ImportInfo {
                    source_file: file_path.to_string(),
                    module_specifier: module_specifier.clone(),
                    imported_name: "*".to_string(),
                    local_name: "*".to_string(),
                    kind: "static".to_string(),
                    is_type_only,
                    line,
                    is_external,
                });
            } else {
                for (imported, local, binding_type_only) in extracted {
                    imports.push(ImportInfo {
                        source_file: file_path.to_string(),
                        module_specifier: module_specifier.clone(),
                        imported_name: imported,
                        local_name: local,
                        kind: "static".to_string(),
                        is_type_only: is_type_only || binding_type_only,
                        line,
                        is_external,
                    });
                }
            }
        } else if has_reexport {
            let reexport_node = reexport_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                .unwrap()
                .node;
            let line = reexport_node.start_position().row as u32;
            let extracted = extract_reexport_bindings(reexport_node, source);
            let is_external = ImportInfo::is_external_specifier(&module_specifier);

            if extracted.is_empty() {
                imports.push(ImportInfo {
                    source_file: file_path.to_string(),
                    module_specifier: module_specifier.clone(),
                    imported_name: "*".to_string(),
                    local_name: "*".to_string(),
                    kind: "re_export".to_string(),
                    is_type_only: has_type_keyword(reexport_node),
                    line,
                    is_external,
                });
            } else {
                for (imported, local) in extracted {
                    imports.push(ImportInfo {
                        source_file: file_path.to_string(),
                        module_specifier: module_specifier.clone(),
                        imported_name: imported,
                        local_name: local,
                        kind: "re_export".to_string(),
                        is_type_only: has_type_keyword(reexport_node),
                        line,
                        is_external,
                    });
                }
            }
        } else if has_dynamic {
            let dynamic_node = dynamic_import_idx
                .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                .unwrap()
                .node;
            imports.push(ImportInfo {
                source_file: file_path.to_string(),
                module_specifier: module_specifier.clone(),
                imported_name: "*".to_string(),
                local_name: "*".to_string(),
                kind: "dynamic".to_string(),
                is_type_only: false,
                line: dynamic_node.start_position().row as u32,
                is_external: ImportInfo::is_external_specifier(&module_specifier),
            });
        } else if has_call {
            let fn_name_cap =
                fn_name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
            if let Some(fn_cap) = fn_name_cap {
                let fn_name = fn_cap.node.utf8_text(source).unwrap_or("");
                if fn_name == "require" {
                    let call_node = call_idx
                        .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                        .unwrap()
                        .node;
                    imports.push(ImportInfo {
                        source_file: file_path.to_string(),
                        module_specifier: module_specifier.clone(),
                        imported_name: "*".to_string(),
                        local_name: "*".to_string(),
                        kind: "require".to_string(),
                        is_type_only: false,
                        line: call_node.start_position().row as u32,
                        is_external: ImportInfo::is_external_specifier(&module_specifier),
                    });
                }
            }
        }
    }

    imports
}

// ── Import helpers ──

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('`') && s.ends_with('`'))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn has_type_keyword(node: tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type" && !child.is_named() {
            return true;
        }
    }
    false
}

fn extract_import_bindings(
    import_node: tree_sitter::Node,
    source: &[u8],
) -> Vec<(String, String, bool)> {
    let mut results = Vec::new();
    let mut cursor = import_node.walk();

    for child in import_node.children(&mut cursor) {
        if child.kind() == "import_clause" {
            extract_import_clause(child, source, &mut results);
        }
    }

    results
}

fn extract_import_clause(
    clause_node: tree_sitter::Node,
    source: &[u8],
    results: &mut Vec<(String, String, bool)>,
) {
    let mut cursor = clause_node.walk();
    for child in clause_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = child.utf8_text(source).unwrap_or("").to_string();
                if !name.is_empty() {
                    results.push(("default".to_string(), name, false));
                }
            }
            "namespace_import" => {
                let local = extract_namespace_local(child, source);
                results.push(("*".to_string(), local, false));
            }
            "named_imports" => {
                extract_named_imports(child, source, results);
            }
            _ => {}
        }
    }
}

fn extract_namespace_local(node: tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return child.utf8_text(source).unwrap_or("*").to_string();
        }
    }
    "*".to_string()
}

fn extract_named_imports(
    node: tree_sitter::Node,
    source: &[u8],
    results: &mut Vec<(String, String, bool)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_specifier" {
            let (imported, local, is_type) = extract_import_specifier(child, source);
            if !imported.is_empty() {
                results.push((imported, local, is_type));
            }
        }
    }
}

fn extract_import_specifier(
    node: tree_sitter::Node,
    source: &[u8],
) -> (String, String, bool) {
    let mut identifiers = Vec::new();
    let mut is_type = false;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "type_identifier" => {
                identifiers.push(child.utf8_text(source).unwrap_or("").to_string());
            }
            "type" => {
                is_type = true;
            }
            _ => {}
        }
    }

    match identifiers.len() {
        0 => (String::new(), String::new(), is_type),
        1 => (identifiers[0].clone(), identifiers[0].clone(), is_type),
        _ => (identifiers[0].clone(), identifiers[1].clone(), is_type),
    }
}

fn extract_reexport_bindings(
    export_node: tree_sitter::Node,
    source: &[u8],
) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut cursor = export_node.walk();

    for child in export_node.children(&mut cursor) {
        if child.kind() == "export_clause" {
            let mut inner_cursor = child.walk();
            for specifier in child.children(&mut inner_cursor) {
                if specifier.kind() == "export_specifier" {
                    let (imported, local) = extract_export_specifier(specifier, source);
                    if !imported.is_empty() {
                        results.push((imported, local));
                    }
                }
            }
        }
    }

    results
}

fn extract_export_specifier(
    node: tree_sitter::Node,
    source: &[u8],
) -> (String, String) {
    let mut identifiers = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            identifiers.push(child.utf8_text(source).unwrap_or("").to_string());
        }
    }

    match identifiers.len() {
        0 => (String::new(), String::new()),
        1 => (identifiers[0].clone(), identifiers[0].clone()),
        _ => (identifiers[0].clone(), identifiers[1].clone()),
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::create_parser;

    // ── Symbol test helpers ──

    fn parse_and_extract(source: &str, language: Language) -> Vec<SymbolInfo> {
        let mut parser = create_parser(language).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(language).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.ts")
    }

    // ── Import test helpers ──

    fn parse_and_extract_imports(source: &str, language: Language) -> Vec<ImportInfo> {
        let mut parser = create_parser(language).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(language).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.ts")
    }

    // ── Symbol tests ──

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
        assert_eq!(syms[0].start_line, 0);
        assert!(syms[0].end_line >= syms[0].start_line);
    }

    // ── Import tests ──

    #[test]
    fn static_named_import() {
        let imports = parse_and_extract_imports(
            r#"import { foo, bar } from "./utils";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].imported_name, "foo");
        assert_eq!(imports[0].local_name, "foo");
        assert_eq!(imports[0].module_specifier, "./utils");
        assert_eq!(imports[0].kind, "static");
        assert!(!imports[0].is_type_only);
        assert!(!imports[0].is_external); // relative path = internal
        assert_eq!(imports[1].imported_name, "bar");
    }

    #[test]
    fn default_import() {
        let imports = parse_and_extract_imports(
            r#"import React from "react";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "default");
        assert_eq!(imports[0].local_name, "React");
        assert_eq!(imports[0].module_specifier, "react");
        assert!(imports[0].is_external); // bare specifier = external
    }

    #[test]
    fn namespace_import() {
        let imports = parse_and_extract_imports(
            r#"import * as path from "path";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "*");
        assert_eq!(imports[0].local_name, "path");
    }

    #[test]
    fn aliased_import() {
        let imports = parse_and_extract_imports(
            r#"import { foo as myFoo } from "./utils";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "foo");
        assert_eq!(imports[0].local_name, "myFoo");
    }

    #[test]
    fn type_only_import() {
        let imports = parse_and_extract_imports(
            r#"import type { User } from "./models";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "User");
        assert!(imports[0].is_type_only);
    }

    #[test]
    fn side_effect_import() {
        let imports = parse_and_extract_imports(
            r#"import "./polyfill";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "*");
        assert_eq!(imports[0].local_name, "*");
        assert_eq!(imports[0].module_specifier, "./polyfill");
    }

    #[test]
    fn dynamic_import() {
        let imports = parse_and_extract_imports(
            r#"const mod = import("./lazy");"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, "dynamic");
        assert_eq!(imports[0].module_specifier, "./lazy");
    }

    #[test]
    fn reexport_star() {
        let imports = parse_and_extract_imports(
            r#"export * from "./base";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, "re_export");
        assert_eq!(imports[0].imported_name, "*");
    }

    #[test]
    fn reexport_named() {
        let imports = parse_and_extract_imports(
            r#"export { foo, bar as baz } from "./helpers";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].kind, "re_export");
        assert_eq!(imports[0].imported_name, "foo");
        assert_eq!(imports[0].local_name, "foo");
        assert_eq!(imports[1].imported_name, "bar");
        assert_eq!(imports[1].local_name, "baz");
    }

    #[test]
    fn require_call() {
        let imports = parse_and_extract_imports(
            r#"const express = require("express");"#,
            Language::JavaScript,
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, "require");
        assert_eq!(imports[0].module_specifier, "express");
        assert!(imports[0].is_external);
    }

    #[test]
    fn non_require_call_ignored() {
        let imports = parse_and_extract_imports(
            r#"const result = fetch("https://api.com");"#,
            Language::JavaScript,
        );
        assert!(imports.is_empty());
    }

    #[test]
    fn default_and_named_combined() {
        let imports = parse_and_extract_imports(
            r#"import React, { useState, useEffect } from "react";"#,
            Language::TypeScript,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].imported_name, "default");
        assert_eq!(imports[0].local_name, "React");
        assert_eq!(imports[1].imported_name, "useState");
        assert_eq!(imports[2].imported_name, "useEffect");
    }

    #[test]
    fn empty_source_no_imports() {
        let imports = parse_and_extract_imports("", Language::TypeScript);
        assert!(imports.is_empty());
    }

    #[test]
    fn strip_quotes_works() {
        assert_eq!(strip_quotes(r#""hello""#), "hello");
        assert_eq!(strip_quotes("'world'"), "world");
        assert_eq!(strip_quotes("`tpl`"), "tpl");
        assert_eq!(strip_quotes("bare"), "bare");
    }

    #[test]
    fn is_external_classification() {
        let source = r#"
import { useState } from "react";
import { helper } from "./utils";
import type { Config } from "@scope/config";
export { foo } from "../shared";
const lazy = import("./lazy-module");
const fs = require("fs");
"#;
        let imports = parse_and_extract_imports(source, Language::TypeScript);
        assert_eq!(imports.len(), 6);

        // react = external
        assert_eq!(imports[0].module_specifier, "react");
        assert!(imports[0].is_external);

        // ./utils = internal
        assert_eq!(imports[1].module_specifier, "./utils");
        assert!(!imports[1].is_external);

        // @scope/config = external
        assert_eq!(imports[2].module_specifier, "@scope/config");
        assert!(imports[2].is_external);

        // ../shared = internal
        assert_eq!(imports[3].module_specifier, "../shared");
        assert!(!imports[3].is_external);

        // ./lazy-module = internal (dynamic)
        assert_eq!(imports[4].module_specifier, "./lazy-module");
        assert!(!imports[4].is_external);

        // fs = external (require)
        assert_eq!(imports[5].module_specifier, "fs");
        assert!(imports[5].is_external);
    }

    #[test]
    fn line_numbers_correct() {
        let source = "// comment\nimport { foo } from \"./bar\";\n";
        let imports = parse_and_extract_imports(source, Language::TypeScript);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].line, 1);
    }
}
