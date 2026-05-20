use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind};

// ── Symbol queries ──

const PYTHON_SYMBOL_QUERY: &str = r#"
(function_definition
  name: (identifier) @name) @definition

(class_definition
  name: (identifier) @name) @definition

(decorated_definition
  definition: (function_definition
    name: (identifier) @name)) @definition

(decorated_definition
  definition: (class_definition
    name: (identifier) @name)) @definition

(expression_statement
  (assignment) @definition)

; Parameters — Python's `parameters` (and `lambda_parameters`) node holds
; several wrapper kinds. For each shape, capture the inner identifier as
; @name and the wrapper itself as @definition so determine_python_kind
; can route it to SymbolKind::Parameter. The `is_parameter_context`
; helper distinguishes parameter identifiers from ordinary expression
; identifiers by walking up to the nearest `parameters`/`lambda_parameters`
; ancestor.
(parameters (identifier) @name @definition)
(parameters (typed_parameter (identifier) @name) @definition)
(parameters (default_parameter name: (identifier) @name) @definition)
(parameters (typed_default_parameter name: (identifier) @name) @definition)
(parameters (list_splat_pattern (identifier) @name) @definition)
(parameters (dictionary_splat_pattern (identifier) @name) @definition)

(lambda_parameters (identifier) @name @definition)
(lambda_parameters (default_parameter name: (identifier) @name) @definition)
(lambda_parameters (list_splat_pattern (identifier) @name) @definition)
(lambda_parameters (dictionary_splat_pattern (identifier) @name) @definition)
"#;

// ── Import queries ──

const PYTHON_IMPORT_QUERY: &str = r#"
(import_statement
  name: (dotted_name) @path) @import

(import_from_statement) @import
"#;

// ── Comment queries ──

const PYTHON_COMMENT_QUERY: &str = r#"
(comment) @comment

(expression_statement
  (string) @docstring)
"#;

// ── Query compilation ──

pub fn compile_symbol_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, PYTHON_SYMBOL_QUERY)
        .with_context(|| format!("failed to compile symbol query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_import_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, PYTHON_IMPORT_QUERY)
        .with_context(|| format!("failed to compile import query for {language}"))?;
    Ok(Arc::new(query))
}

pub fn compile_comment_query(language: Language) -> Result<Arc<Query>> {
    let ts_lang = language.tree_sitter_language();
    let query = Query::new(&ts_lang, PYTHON_COMMENT_QUERY)
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
    // (enclosing_function_id, name) seen for local-variable dedupe — keep
    // only the FIRST assignment per name per function scope, per the
    // issue #11 acceptance criterion ("first assignment within a function
    // scope"). Module-level assignments are not deduped (the existing
    // module-scope path handles those before this map is consulted).
    let mut seen_locals: HashSet<(usize, String)> = HashSet::new();
    // Collect candidate rows in source order then materialise — we need
    // source-order traversal so the FIRST assignment wins.
    let mut candidates: Vec<(tree_sitter::Node, String)> = Vec::new();

    while let Some(m) = matches.next() {
        let def_cap = definition_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let Some(def_cap) = def_cap else {
            continue;
        };
        let def_node = def_cap.node;

        // For assignment nodes, extract name from `left` field manually
        let name = if def_node.kind() == "assignment" {
            let left = def_node.child_by_field_name("left");
            match left {
                Some(n) if n.kind() == "identifier" => {
                    n.utf8_text(source).unwrap_or("").to_string()
                }
                _ => continue, // skip attribute, subscript, destructuring
            }
        } else {
            let name_cap = name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
            let Some(name_cap) = name_cap else {
                continue;
            };
            name_cap.node.utf8_text(source).unwrap_or("").to_string()
        };

        if name.is_empty() {
            continue;
        }

        // Skip function_definition/class_definition whose parent is decorated_definition
        // (the decorated_definition match will handle them)
        if (def_node.kind() == "function_definition" || def_node.kind() == "class_definition")
            && def_node
                .parent()
                .is_some_and(|p| p.kind() == "decorated_definition")
        {
            continue;
        }

        candidates.push((def_node, name));
    }

    // Sort by start_byte so the "first assignment wins" dedupe is
    // deterministic regardless of tree-sitter match order.
    candidates.sort_by_key(|(n, _)| n.start_byte());

    for (def_node, name) in candidates {
        let kind = determine_python_kind(def_node, &name);
        let Some(kind) = kind else { continue };

        // Function-local assignment dedupe: only the first `name = ...`
        // inside a given function counts as the binding site. Subsequent
        // re-assignments are writes against the same binding; they don't
        // get their own symbol row.
        if def_node.kind() == "assignment" && kind == SymbolKind::Variable {
            if let Some(scope) = enclosing_function_node(def_node) {
                let key = (scope.id(), name.clone());
                if !seen_locals.insert(key) {
                    continue;
                }
            }
        }

        let is_exported = !name.starts_with('_');

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
        };
        symbols.push(symbol);
    }

    symbols
}

/// Walk up from `node` to the nearest enclosing `function_definition` or
/// `lambda` node. Returns `None` if `node` is at module scope. Used to
/// scope-key the "first assignment" dedupe.
fn enclosing_function_node(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "function_definition" | "lambda" => return Some(parent),
            _ => current = parent.parent(),
        }
    }
    None
}

fn determine_python_kind(def_node: tree_sitter::Node, _name: &str) -> Option<SymbolKind> {
    match def_node.kind() {
        "function_definition" => {
            if is_inside_class(def_node) {
                Some(SymbolKind::Method)
            } else {
                Some(SymbolKind::Function)
            }
        }
        "class_definition" => Some(SymbolKind::Class),
        "decorated_definition" => {
            // Look at the inner definition
            let inner = def_node.child_by_field_name("definition")?;
            match inner.kind() {
                "function_definition" => {
                    if is_inside_class(def_node) {
                        Some(SymbolKind::Method)
                    } else {
                        Some(SymbolKind::Function)
                    }
                }
                "class_definition" => Some(SymbolKind::Class),
                _ => None,
            }
        }
        "assignment" => {
            // Module-level assignment (Tree: module > expression_statement >
            // assignment) AND function-local assignment both bind a name.
            // The extractor emits a Variable symbol for either; the
            // function-local case is deduped upstream so only the FIRST
            // assignment per (function, name) becomes a symbol row. Per
            // ADR-0005 / docs/references-python.md, this is the binding
            // site for the local name in its function scope.
            //
            // Skip assignments inside class bodies whose direct enclosing
            // block is a class body — those are class attributes, not
            // function locals or module-level vars. (They still get a
            // Variable kind today, matching pre-Phase-2 behaviour.)
            let parent = def_node.parent(); // expression_statement
            let grandparent = parent.and_then(|p| p.parent()); // module or block
            match grandparent.map(|g| g.kind()) {
                Some("module") => Some(SymbolKind::Variable),
                Some("block") => {
                    // block parent could be a function body or a class body
                    // — only function bodies yield locals.
                    if enclosing_function_node(def_node).is_some() {
                        Some(SymbolKind::Variable)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        // Parameter wrappers from the symbol query — all map to Parameter.
        "identifier" => {
            // A bare identifier reaches this match only via the
            // `(parameters (identifier) @name @definition)` /
            // `(lambda_parameters ...)` patterns, so its parent is
            // guaranteed to be one of those.
            Some(SymbolKind::Parameter)
        }
        "typed_parameter"
        | "default_parameter"
        | "typed_default_parameter"
        | "list_splat_pattern"
        | "dictionary_splat_pattern" => Some(SymbolKind::Parameter),
        _ => None,
    }
}

fn is_inside_class(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_definition" => return true,
            "function_definition" => return false, // nested function, not a method
            _ => {
                current = parent.parent();
            }
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
        let import_cap = import_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let Some(import_cap) = import_cap else {
            continue;
        };

        let import_node = import_cap.node;
        let line = import_node.start_position().row as u32 + 1;

        match import_node.kind() {
            "import_statement" => {
                // import foo, import foo.bar
                let path_cap = path_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
                if let Some(path_cap) = path_cap {
                    let module = path_cap.node.utf8_text(source).unwrap_or("").to_string();
                    if module.is_empty() {
                        continue;
                    }

                    let imported_name = module.rsplit('.').next().unwrap_or(&module).to_string();

                    imports.push(ImportInfo {
                        source_file: file_path.to_string(),
                        module_specifier: module,
                        imported_name: imported_name.clone(),
                        local_name: imported_name,
                        kind: "import".to_string(),
                        is_type_only: false,
                        line,
                        is_external: true,
                    });
                }
            }
            "import_from_statement" => {
                // from foo import bar, baz
                let module = extract_from_module(import_node, source);

                let is_internal = module.starts_with('.');

                // Collect imported names
                let mut cursor_walk = import_node.walk();
                let mut found_names = false;
                for child in import_node.children(&mut cursor_walk) {
                    match child.kind() {
                        "dotted_name"
                            if found_names || is_import_name_position(import_node, child) =>
                        {
                            // Skip the module part (first dotted_name is the module)
                            // Subsequent dotted_names are imported names
                            let name = child.utf8_text(source).unwrap_or("").to_string();
                            if !name.is_empty() && name != module {
                                imports.push(ImportInfo {
                                    source_file: file_path.to_string(),
                                    module_specifier: module.clone(),
                                    imported_name: name.clone(),
                                    local_name: name,
                                    kind: "from".to_string(),
                                    is_type_only: false,
                                    line,
                                    is_external: !is_internal,
                                });
                            }
                        }
                        "aliased_import" => {
                            found_names = true;
                            let name_node = child.child_by_field_name("name");
                            let alias_node = child.child_by_field_name("alias");
                            if let Some(name_node) = name_node {
                                let name = name_node.utf8_text(source).unwrap_or("").to_string();
                                let local = alias_node
                                    .and_then(|n| n.utf8_text(source).ok())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| name.clone());

                                if !name.is_empty() {
                                    imports.push(ImportInfo {
                                        source_file: file_path.to_string(),
                                        module_specifier: module.clone(),
                                        imported_name: name,
                                        local_name: local,
                                        kind: "from".to_string(),
                                        is_type_only: false,
                                        line,
                                        is_external: !is_internal,
                                    });
                                }
                            }
                        }
                        "wildcard_import" => {
                            found_names = true;
                            imports.push(ImportInfo {
                                source_file: file_path.to_string(),
                                module_specifier: module.clone(),
                                imported_name: "*".to_string(),
                                local_name: "*".to_string(),
                                kind: "from".to_string(),
                                is_type_only: false,
                                line,
                                is_external: !is_internal,
                            });
                        }
                        "import" => {
                            // The "import" keyword marks the transition to imported names
                            found_names = true;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    imports
}

fn extract_from_module(import_node: tree_sitter::Node, source: &[u8]) -> String {
    // Try module_name field first
    if let Some(module_node) = import_node.child_by_field_name("module_name") {
        return module_node.utf8_text(source).unwrap_or("").to_string();
    }

    // Look for dotted_name or relative_import before the "import" keyword
    let mut cursor = import_node.walk();
    let mut found_from = false;
    for child in import_node.children(&mut cursor) {
        if child.kind() == "from" {
            found_from = true;
            continue;
        }
        if found_from && child.kind() == "import" {
            break;
        }
        if found_from {
            match child.kind() {
                "dotted_name" | "relative_import" => {
                    return child.utf8_text(source).unwrap_or("").to_string();
                }
                _ => {}
            }
        }
    }

    String::new()
}

fn is_import_name_position(import_node: tree_sitter::Node, name_node: tree_sitter::Node) -> bool {
    // Check if this dotted_name comes after the "import" keyword
    let mut cursor = import_node.walk();
    let mut past_import_keyword = false;
    for child in import_node.children(&mut cursor) {
        if child.kind() == "import" {
            past_import_keyword = true;
            continue;
        }
        if past_import_keyword && child.id() == name_node.id() {
            return true;
        }
    }
    false
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
    let docstring_idx = query.capture_index_for_name("docstring");

    let mut comments = Vec::new();

    while let Some(m) = matches.next() {
        // Handle regular comments
        if let Some(comment_cap) =
            comment_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx))
        {
            let node = comment_cap.node;
            let text = node.utf8_text(source).unwrap_or("").to_string();
            if text.is_empty() {
                continue;
            }

            let (associated_symbol, associated_symbol_kind) = find_associated_symbol(node, source);

            comments.push(CommentInfo {
                file_path: file_path.to_string(),
                text,
                kind: "line".to_string(),
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
                start_line: node.start_position().row as u32 + 1,
                start_column: node.start_position().column as u32,
                end_line: node.end_position().row as u32 + 1,
                end_column: node.end_position().column as u32,
                associated_symbol,
                associated_symbol_kind,
            });
            continue;
        }

        // Handle docstrings
        if let Some(docstring_cap) =
            docstring_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx))
        {
            let node = docstring_cap.node;
            let text = node.utf8_text(source).unwrap_or("").to_string();
            if text.is_empty() {
                continue;
            }

            // Check if this is actually a docstring (first statement in function/class/module body)
            let is_docstring = is_docstring_position(node);

            if is_docstring {
                let (associated_symbol, associated_symbol_kind) =
                    find_docstring_symbol(node, source);

                comments.push(CommentInfo {
                    file_path: file_path.to_string(),
                    text,
                    kind: "doc".to_string(),
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
        }
    }

    comments
}

fn is_docstring_position(string_node: tree_sitter::Node) -> bool {
    // The string must be inside an expression_statement
    let Some(expr_stmt) = string_node.parent() else {
        return false;
    };
    if expr_stmt.kind() != "expression_statement" {
        return false;
    }

    // The expression_statement must be the first statement in a block
    let Some(parent) = expr_stmt.parent() else {
        return false;
    };

    match parent.kind() {
        "module" => {
            // Module-level docstring: first expression_statement in module
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.is_named() {
                    return child.id() == expr_stmt.id();
                }
            }
            false
        }
        "block" => {
            // Function/class body docstring: first statement in block
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.is_named() {
                    return child.id() == expr_stmt.id();
                }
            }
            false
        }
        _ => false,
    }
}

fn find_docstring_symbol(
    string_node: tree_sitter::Node,
    source: &[u8],
) -> (Option<String>, Option<String>) {
    // Walk up: string → expression_statement → block → function_definition/class_definition
    let expr_stmt = string_node.parent();
    let block = expr_stmt.and_then(|n| n.parent());
    let container = block.and_then(|n| n.parent());

    let Some(container) = container else {
        return (None, None);
    };

    match container.kind() {
        "function_definition" => {
            let name = container
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            let kind = if is_inside_class(container) {
                "method"
            } else {
                "function"
            };
            (name, Some(kind.to_string()))
        }
        "class_definition" => {
            let name = container
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("class".to_string()))
        }
        _ => (None, None),
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
            let kind = if is_inside_class(node) {
                "method"
            } else {
                "function"
            };
            (name, Some(kind.to_string()))
        }
        "class_definition" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            (name, Some("class".to_string()))
        }
        "decorated_definition" => {
            // Unwrap to inner definition
            if let Some(inner) = node.child_by_field_name("definition") {
                return extract_symbol_from_node(inner, source);
            }
            (None, None)
        }
        _ => (None, None),
    }
}

// ── Import resolution ──

/// Resolve a relative Python import to a file path.
/// Handles leading dots for relative depth.
pub fn resolve_import(
    source_file: &str,
    specifier: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    // Count leading dots
    let dots = specifier.chars().take_while(|c| *c == '.').count();
    if dots == 0 {
        return None; // Absolute import, treated as external
    }

    let remaining = &specifier[dots..];
    let dir = source_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };

    // Each dot beyond the first goes up one directory
    for _ in 1..dots {
        parts.pop();
    }

    // Add remaining path segments (dotted)
    if !remaining.is_empty() {
        for segment in remaining.split('.') {
            parts.push(segment);
        }
    }

    let path = parts.join("/");

    // Try as a module file
    let py_path = format!("{}.py", path);
    if known_files.contains(&py_path) {
        return Some(py_path);
    }

    // Try as a package (__init__.py)
    let init_path = format!("{}/__init__.py", path);
    if known_files.contains(&init_path) {
        return Some(init_path);
    }

    // Try .pyi stub
    let pyi_path = format!("{}.pyi", path);
    if known_files.contains(&pyi_path) {
        return Some(pyi_path);
    }

    None
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::create_parser;

    fn parse_and_extract(source: &str) -> Vec<SymbolInfo> {
        let mut parser = create_parser(Language::Python).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_symbol_query(Language::Python).expect("compile query");
        extract_symbols(&tree, source.as_bytes(), &query, "test.py")
    }

    fn parse_and_extract_imports(source: &str) -> Vec<ImportInfo> {
        let mut parser = create_parser(Language::Python).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_import_query(Language::Python).expect("compile import query");
        extract_imports(&tree, source.as_bytes(), &query, "test.py")
    }

    fn parse_and_extract_comments(source: &str) -> Vec<CommentInfo> {
        let mut parser = create_parser(Language::Python).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let query = compile_comment_query(Language::Python).expect("compile comment query");
        extract_comments(&tree, source.as_bytes(), &query, "test.py")
    }

    #[test]
    fn extract_function() {
        let syms = parse_and_extract("def hello():\n    pass");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "hello");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert!(syms[0].is_exported);
    }

    #[test]
    fn extract_private_function() {
        let syms = parse_and_extract("def _helper():\n    pass");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "_helper");
        assert!(!syms[0].is_exported);
    }

    #[test]
    fn extract_class() {
        let syms = parse_and_extract("class Foo:\n    pass");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Foo");
        assert_eq!(syms[0].kind, SymbolKind::Class);
    }

    #[test]
    fn extract_method() {
        let syms = parse_and_extract("class Foo:\n    def bar(self):\n        pass");
        let method = syms.iter().find(|s| s.name == "bar");
        assert!(method.is_some());
        assert_eq!(method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn extract_decorated_function() {
        let syms = parse_and_extract("@decorator\ndef hello():\n    pass");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "hello");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn extract_module_variable() {
        let syms = parse_and_extract("MAX_SIZE = 100");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MAX_SIZE");
        assert_eq!(syms[0].kind, SymbolKind::Variable);
    }

    #[test]
    fn import_statement() {
        let imports = parse_and_extract_imports("import os");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "os");
        assert_eq!(imports[0].kind, "import");
        assert!(imports[0].is_external);
    }

    #[test]
    fn from_import() {
        let imports = parse_and_extract_imports("from os import path");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_specifier, "os");
        assert_eq!(imports[0].imported_name, "path");
        assert_eq!(imports[0].kind, "from");
    }

    #[test]
    fn relative_import() {
        let imports = parse_and_extract_imports("from . import utils");
        assert!(!imports.is_empty());
        assert!(!imports[0].is_external);
    }

    #[test]
    fn line_comment() {
        let comments = parse_and_extract_comments("# This is a comment");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, "line");
    }

    #[test]
    fn docstring() {
        let comments = parse_and_extract_comments(
            "def foo():\n    \"\"\"This is a docstring.\"\"\"\n    pass",
        );
        let doc = comments.iter().find(|c| c.kind == "doc");
        assert!(doc.is_some());
        assert_eq!(doc.unwrap().associated_symbol.as_deref(), Some("foo"));
    }

    #[test]
    fn empty_source_no_symbols() {
        let syms = parse_and_extract("");
        assert!(syms.is_empty());
    }

    // ── Issue #11 Phase 2: parameter + local-variable extraction ──

    #[test]
    fn extract_parameters_positional_and_typed() {
        let syms = parse_and_extract("def greet(name, age: int):\n    pass");
        let params: Vec<&SymbolInfo> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .collect();
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"name"), "expected `name` param, got {names:?}");
        assert!(names.contains(&"age"), "expected `age` param, got {names:?}");
    }

    #[test]
    fn extract_parameters_defaults_and_splats() {
        // covers default_parameter, typed_default_parameter,
        // list_splat_pattern (*args), dictionary_splat_pattern (**kwargs).
        let syms = parse_and_extract(
            "def f(a, b=1, c: int = 2, *args, **kwargs):\n    pass",
        );
        let names: Vec<&str> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .map(|s| s.name.as_str())
            .collect();
        for expected in ["a", "b", "c", "args", "kwargs"] {
            assert!(
                names.contains(&expected),
                "expected param `{expected}`, got {names:?}"
            );
        }
    }

    #[test]
    fn extract_function_local_variable() {
        let syms = parse_and_extract("def f():\n    x = 1\n    return x");
        let locals: Vec<&SymbolInfo> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable && s.name == "x")
            .collect();
        assert_eq!(locals.len(), 1, "expected exactly one local symbol for `x`");
    }

    #[test]
    fn dedupe_local_variable_first_assignment_only() {
        // Two assignments to the same name inside the same function — only
        // the first should produce a symbol row (per issue #11 #3).
        let syms = parse_and_extract(
            "def f():\n    x = 1\n    x = 2\n    return x",
        );
        let locals: Vec<&SymbolInfo> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable && s.name == "x")
            .collect();
        assert_eq!(locals.len(), 1, "second assignment must not emit a new symbol");
        assert_eq!(locals[0].start_line, 2, "first assignment wins");
    }

    #[test]
    fn decorated_function_still_emits_parameters_once() {
        // Guards against the decorated_definition dedupe accidentally
        // suppressing parameter symbols (or duplicating them).
        let syms = parse_and_extract("@dec\ndef f(x, y):\n    pass");
        let fn_count = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "f")
            .count();
        assert_eq!(fn_count, 1, "decorated function should be emitted once");
        let params: Vec<&str> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .map(|s| s.name.as_str())
            .collect();
        assert!(params.contains(&"x") && params.contains(&"y"), "got {params:?}");
    }
}
