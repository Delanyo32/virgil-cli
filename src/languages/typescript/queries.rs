use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::language::Language;
use crate::models::{CommentInfo, ImportInfo, SymbolInfo, SymbolKind, SymbolVisibility};

/// Classify the visibility of a TS/JS definition.
///
/// - Parameters → Private.
/// - Class members (`method_definition`, `public_field_definition`):
///   read the `accessibility_modifier` child. `public`/`private`/
///   `protected` map directly; absent modifier → Public (TS default).
/// - Top-level symbols: Public iff `is_exported`, else Private.
fn visibility_ts(
    def_node: tree_sitter::Node,
    kind: SymbolKind,
    is_exported: bool,
    source: &[u8],
) -> SymbolVisibility {
    if kind == SymbolKind::Parameter {
        return SymbolVisibility::Private;
    }
    if is_class_member(def_node) {
        if let Some(modifier) = find_accessibility_modifier(def_node, source) {
            return match modifier.as_str() {
                "private" => SymbolVisibility::Private,
                "protected" => SymbolVisibility::Protected,
                _ => SymbolVisibility::Public,
            };
        }
        return SymbolVisibility::Public;
    }
    if is_exported {
        SymbolVisibility::Public
    } else {
        SymbolVisibility::Private
    }
}

/// True when `def_node` is a class-body member: `method_definition`,
/// `public_field_definition`, `method_signature`, or `abstract_method_signature`.
/// Excludes object-literal methods (parent is `object`, not `class_body`).
fn is_class_member(def_node: tree_sitter::Node) -> bool {
    let kind_ok = matches!(
        def_node.kind(),
        "method_definition"
            | "public_field_definition"
            | "method_signature"
            | "abstract_method_signature"
    );
    if !kind_ok {
        return false;
    }
    matches!(
        def_node.parent().map(|p| p.kind()),
        Some("class_body") | Some("interface_body")
    )
}

/// Read the literal text of the `accessibility_modifier` child, if any.
fn find_accessibility_modifier(def_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = def_node.walk();
    for child in def_node.children(&mut cursor) {
        if child.kind() == "accessibility_modifier" {
            return Some(child.utf8_text(source).unwrap_or("").trim().to_string());
        }
    }
    None
}

/// True if any direct child is an `async` anonymous keyword token.
fn has_keyword_child(node: tree_sitter::Node, keyword: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() && child.kind() == keyword {
            return true;
        }
    }
    false
}

/// True if the symbol carries the `async` keyword. Checks the def node
/// itself (function/method declarations) and, for variable bindings,
/// the bound value (e.g. `const f = async () => ...`).
fn is_async_ts(def_node: tree_sitter::Node, value_node: Option<tree_sitter::Node>) -> bool {
    let async_targets = [
        "function_declaration",
        "function_expression",
        "arrow_function",
        "method_definition",
        "method_signature",
        "generator_function",
        "generator_function_declaration",
    ];
    if async_targets.contains(&def_node.kind()) && has_keyword_child(def_node, "async") {
        return true;
    }
    if let Some(v) = value_node
        && async_targets.contains(&v.kind())
        && has_keyword_child(v, "async")
    {
        return true;
    }
    false
}

/// True if the class member has a `static` keyword child.
fn is_static_ts(def_node: tree_sitter::Node) -> bool {
    if !is_class_member(def_node) {
        return false;
    }
    has_keyword_child(def_node, "static")
}

/// True if the def is an `abstract_class_declaration`, or a class
/// member with the `abstract` keyword.
fn is_abstract_ts(def_node: tree_sitter::Node) -> bool {
    if def_node.kind() == "abstract_class_declaration" {
        return true;
    }
    if is_class_member(def_node) && has_keyword_child(def_node, "abstract") {
        return true;
    }
    // tree-sitter-typescript also exposes `abstract_method_signature`.
    if def_node.kind() == "abstract_method_signature" {
        return true;
    }
    false
}

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

(required_parameter
  pattern: (identifier) @name) @parameter

(optional_parameter
  pattern: (identifier) @name) @parameter

(required_parameter
  pattern: (rest_pattern (identifier) @name)) @parameter

(arrow_function
  parameter: (identifier) @name) @parameter

(public_field_definition
  name: (property_identifier) @name) @definition

(property_signature
  name: (property_identifier) @name) @definition
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

(formal_parameters
  (identifier) @name) @parameter

(formal_parameters
  (rest_pattern (identifier) @name)) @parameter

(formal_parameters
  (assignment_pattern
    left: (identifier) @name)) @parameter

(arrow_function
  parameter: (identifier) @name) @parameter
"#;

/// Matches `exports.NAME = VALUE` and `module.exports.NAME = VALUE` assignments.
/// Used for a second-pass CommonJS export detection on JavaScript/JSX files.
const JS_COMMONJS_EXPORT_QUERY: &str = r#"
(assignment_expression
  left: (member_expression
    object: (member_expression
      object: (identifier) @module_obj
      property: (property_identifier) @exports_kw)
    property: (property_identifier) @name)
  right: (_) @value) @assign

(assignment_expression
  left: (member_expression
    object: (identifier) @exports_obj
    property: (property_identifier) @name)
  right: (_) @value) @assign
"#;

/// `module.exports = { foo, bar }` and `exports = { foo }` — object-literal
/// re-exports. Each property key marks an already-defined symbol exported.
/// (Branch text guards — `module`/`exports` — are checked in Rust.)
const JS_COMMONJS_OBJECT_EXPORT_QUERY: &str = r#"
(assignment_expression
  left: (member_expression
    object: (identifier) @mod
    property: (property_identifier) @exp)
  right: (object) @obj)

(assignment_expression
  left: (identifier) @exports_ident
  right: (object) @obj)
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

// ── Comment queries ──

const COMMENT_QUERY: &str = r#"
(comment) @comment
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
    language: Language,
) -> Vec<SymbolInfo> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let name_idx = query.capture_index_for_name("name");
    let definition_idx = query.capture_index_for_name("definition");
    let value_idx = query.capture_index_for_name("value");
    let parameter_idx = query.capture_index_for_name("parameter");

    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        let name_cap = name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let def_cap = definition_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let value_cap = value_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
        let param_cap = parameter_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));

        let Some(name_cap) = name_cap else { continue };

        let name_node = name_cap.node;
        let name = name_node.utf8_text(source).unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }

        // Parameter capture takes precedence — anchor the symbol on the identifier
        // node itself (parameter containers in JS are formal_parameters, which spans
        // every parameter; using the name node keeps the byte range per-parameter).
        let (def_node, kind, is_exported) = if let Some(_param_cap) = param_cap {
            (name_node, SymbolKind::Parameter, false)
        } else {
            let Some(def_cap) = def_cap else { continue };
            let def_node = def_cap.node;
            let kind = determine_kind(def_node.kind(), value_cap.map(|c| c.node.kind()));
            let Some(kind) = kind else { continue };
            // Check if parent is an export_statement
            let is_exported = def_node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement");
            (def_node, kind, is_exported)
        };

        let value_node = value_cap.map(|c| c.node);
        let visibility = visibility_ts(def_node, kind, is_exported, source);
        let is_async = is_async_ts(def_node, value_node);
        let is_static = is_static_ts(def_node);
        let is_abstract = is_abstract_ts(def_node);

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
            is_async,
            is_static,
            is_abstract,
            // TS `readonly` lives in `typescript_attrs.is_readonly`, not here.
            is_mutable: false,
        };
        symbols.push(symbol);
    }

    // Second pass: detect CommonJS exports (exports.NAME = fn / module.exports.NAME = fn)
    // Only applies to JavaScript and JSX files.
    if matches!(language, Language::JavaScript | Language::Jsx) {
        let ts_lang = language.tree_sitter_language();
        if let Ok(cjs_query) = tree_sitter::Query::new(&ts_lang, JS_COMMONJS_EXPORT_QUERY) {
            let name_idx = cjs_query.capture_index_for_name("name");
            let value_idx = cjs_query.capture_index_for_name("value");
            let assign_idx = cjs_query.capture_index_for_name("assign");
            let module_obj_idx = cjs_query.capture_index_for_name("module_obj");
            let exports_kw_idx = cjs_query.capture_index_for_name("exports_kw");

            let mut cjs_cursor = tree_sitter::QueryCursor::new();
            let mut cjs_matches = cjs_cursor.matches(&cjs_query, tree.root_node(), source);

            while let Some(m) = cjs_matches.next() {
                // For `module.exports.NAME = VALUE`, guard that object is "module" and
                // property is "exports". For bare `exports.NAME = VALUE` the query
                // already constrains via @exports_obj (#eq? is not used here, so we
                // must check the text ourselves).
                let name_cap = name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
                let value_cap =
                    value_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));
                let assign_cap =
                    assign_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx));

                let Some(name_cap) = name_cap else { continue };

                let export_name = match name_cap.node.utf8_text(source) {
                    Ok(n) if !n.is_empty() => n.to_string(),
                    _ => continue,
                };

                // If this is a module.exports.NAME pattern, verify the object names.
                // The query captures @module_obj and @exports_kw only for that branch.
                let is_module_exports_branch = module_obj_idx
                    .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                    .is_some();

                if is_module_exports_branch {
                    let module_text = module_obj_idx
                        .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                        .and_then(|c| c.node.utf8_text(source).ok())
                        .unwrap_or("");
                    let exports_text = exports_kw_idx
                        .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                        .and_then(|c| c.node.utf8_text(source).ok())
                        .unwrap_or("");
                    if module_text != "module" || exports_text != "exports" {
                        continue;
                    }
                } else {
                    // Bare `exports.NAME = VALUE` branch: guard that the object is "exports".
                    // The @exports_obj capture is used in the query for the bare branch.
                    let exports_obj_idx = cjs_query.capture_index_for_name("exports_obj");
                    let obj_text = exports_obj_idx
                        .and_then(|idx| m.captures.iter().find(|c| c.index == idx))
                        .and_then(|c| c.node.utf8_text(source).ok())
                        .unwrap_or("");
                    if obj_text != "exports" {
                        continue;
                    }
                }

                let value_kind = value_cap.map(|c| c.node.kind());
                let symbol_kind = match value_kind {
                    // tree-sitter-javascript uses "function_expression" for
                    // named/anonymous function expressions (including async ones).
                    // "function" is just the keyword token (unnamed node).
                    Some("function_expression") | Some("function") => SymbolKind::Function,
                    Some("generator_function") | Some("generator_function_expression") => {
                        SymbolKind::Function
                    }
                    Some("arrow_function") => SymbolKind::ArrowFunction,
                    _ => SymbolKind::Variable,
                };

                let start_line = name_cap.node.start_position().row as u32 + 1;
                let end_line = assign_cap
                    .map(|c| c.node.end_position().row as u32 + 1)
                    .unwrap_or(start_line);

                // If a symbol with the same name already exists, just mark it exported.
                if let Some(existing) = symbols.iter_mut().find(|s| s.name == export_name) {
                    existing.is_exported = true;
                } else {
                    symbols.push(SymbolInfo {
                        name: export_name,
                        kind: symbol_kind,
                        file_path: file_path.to_string(),
                        start_byte: name_cap.node.start_byte() as u32,
                        end_byte: name_cap.node.end_byte() as u32,
                        start_line,
                        start_column: name_cap.node.start_position().column as u32,
                        end_line,
                        end_column: name_cap.node.end_position().column as u32,
                        is_exported: true,
                        visibility: SymbolVisibility::Public,
                        is_async: false,
                        is_static: false,
                        is_abstract: false,
                        is_mutable: false,
                    });
                }
            }
        }

        // Third pass: object-literal re-exports `module.exports = { foo, bar }`
        // / `exports = { foo }`. Each property key flags an already-extracted
        // symbol exported (the common Express controller/service/util shape).
        if let Ok(obj_query) = tree_sitter::Query::new(&ts_lang, JS_COMMONJS_OBJECT_EXPORT_QUERY) {
            let obj_idx = obj_query.capture_index_for_name("obj");
            let mod_idx = obj_query.capture_index_for_name("mod");
            let exp_idx = obj_query.capture_index_for_name("exp");
            let ident_idx = obj_query.capture_index_for_name("exports_ident");
            let cap_text = |m: &tree_sitter::QueryMatch, idx: Option<u32>| -> Option<String> {
                idx.and_then(|i| m.captures.iter().find(|c| c.index == i))
                    .and_then(|c| c.node.utf8_text(source).ok())
                    .map(|s| s.to_string())
            };

            let mut obj_cursor = tree_sitter::QueryCursor::new();
            let mut obj_matches = obj_cursor.matches(&obj_query, tree.root_node(), source);
            while let Some(m) = obj_matches.next() {
                // Guard the assignment target is really `module.exports` / `exports`.
                let is_target = match cap_text(m, mod_idx) {
                    Some(mo) => {
                        mo == "module" && cap_text(m, exp_idx).as_deref() == Some("exports")
                    }
                    None => cap_text(m, ident_idx).as_deref() == Some("exports"),
                };
                if !is_target {
                    continue;
                }
                let Some(obj_cap) = obj_idx.and_then(|i| m.captures.iter().find(|c| c.index == i))
                else {
                    continue;
                };
                let obj_node = obj_cap.node;
                let mut oc = obj_node.walk();
                for member in obj_node.named_children(&mut oc) {
                    let name = match member.kind() {
                        "shorthand_property_identifier" => member.utf8_text(source).ok(),
                        "pair" => member
                            .child_by_field_name("key")
                            .and_then(|k| k.utf8_text(source).ok()),
                        _ => None,
                    };
                    if let Some(name) = name.filter(|n| !n.is_empty())
                        && let Some(existing) = symbols.iter_mut().find(|s| s.name == name)
                    {
                        existing.is_exported = true;
                    }
                }
            }
        }
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
        "public_field_definition" | "property_signature" => Some(SymbolKind::Field),
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
            let line = import_node.start_position().row as u32 + 1;
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
            let line = reexport_node.start_position().row as u32 + 1;
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
                line: dynamic_node.start_position().row as u32 + 1,
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
                        line: call_node.start_position().row as u32 + 1,
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
        s[1..s.len() - 1].trim().to_string()
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

fn extract_import_specifier(node: tree_sitter::Node, source: &[u8]) -> (String, String, bool) {
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

fn extract_export_specifier(node: tree_sitter::Node, source: &[u8]) -> (String, String) {
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
    let kind_str = node.kind();

    // If the sibling is an export_statement, look at its first named child
    if kind_str == "export_statement" {
        if let Some(child) = node.named_child(0) {
            return extract_symbol_from_node(child, source);
        }
        return (None, None);
    }

    let symbol_kind = match kind_str {
        "function_declaration" => "function",
        "class_declaration" => "class",
        "method_definition" => "method",
        "interface_declaration" => "interface",
        "type_alias_declaration" => "type_alias",
        "enum_declaration" => "enum",
        "lexical_declaration" | "variable_declaration" => {
            // Drill into variable_declarator to get the name
            if let Some(declarator) = find_child_by_kind(node, "variable_declarator") {
                let name = declarator
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string());
                let value_kind = declarator.child_by_field_name("value").map(|n| n.kind());
                let sk = if value_kind == Some("arrow_function") {
                    "arrow_function"
                } else {
                    "variable"
                };
                return (name, Some(sk.to_string()));
            }
            return (None, None);
        }
        _ => return (None, None),
    };

    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .map(|s| s.to_string());

    (name, Some(symbol_kind.to_string()))
}

fn find_child_by_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

// ── Import resolution ──

/// Resolve a relative import specifier to a file path in the workspace.
/// Tries extension inference (.ts, .tsx, .js, .jsx) and index file fallback.
pub fn resolve_import(
    source_file: &str,
    specifier: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    let base_dir = source_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    // Normalize the relative path
    let resolved = normalize_relative_path(base_dir, specifier);

    // Try exact match first
    if known_files.contains(&resolved) {
        return Some(resolved);
    }

    // NodeNext / ESM: specifiers like "./foo.js" map to "./foo.ts" on disk
    for (from, to) in &[
        (".js", ".ts"),
        (".jsx", ".tsx"),
        (".mjs", ".mts"),
        (".cjs", ".cts"),
    ] {
        if let Some(stem) = resolved.strip_suffix(from) {
            let candidate = format!("{stem}{to}");
            if known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
    }

    // Try extensions
    for ext in &[".ts", ".tsx", ".js", ".jsx"] {
        let candidate = format!("{}{}", resolved, ext);
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    }

    // Try index files
    for ext in &[".ts", ".tsx", ".js", ".jsx"] {
        let candidate = format!("{}/index{}", resolved, ext);
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

/// Normalize a relative path (./foo, ../bar) against a base directory.
fn normalize_relative_path(base_dir: &str, specifier: &str) -> String {
    let specifier = specifier.strip_prefix("./").unwrap_or(specifier);

    let mut parts: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };

    for segment in specifier.split('/') {
        match segment {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            other => parts.push(other),
        }
    }

    parts.join("/")
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
        extract_symbols(&tree, source.as_bytes(), &query, "test.ts", language)
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
        assert_eq!(
            determine_kind("function_declaration", None),
            Some(SymbolKind::Function)
        );
    }

    #[test]
    fn determine_kind_class() {
        assert_eq!(
            determine_kind("class_declaration", None),
            Some(SymbolKind::Class)
        );
    }

    #[test]
    fn determine_kind_method() {
        assert_eq!(
            determine_kind("method_definition", None),
            Some(SymbolKind::Method)
        );
    }

    #[test]
    fn determine_kind_interface() {
        assert_eq!(
            determine_kind("interface_declaration", None),
            Some(SymbolKind::Interface)
        );
    }

    #[test]
    fn determine_kind_type_alias() {
        assert_eq!(
            determine_kind("type_alias_declaration", None),
            Some(SymbolKind::TypeAlias)
        );
    }

    #[test]
    fn determine_kind_enum() {
        assert_eq!(
            determine_kind("enum_declaration", None),
            Some(SymbolKind::Enum)
        );
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
    fn test_commonjs_exports_symbolized_with_correct_kind() {
        let source = r#"exports.createComment = async function(req, res) {
    return res.json({});
};
exports.updatePost = (req, res) => {
    return res.json({});
};
module.exports.deleteItem = function(req, res) {
    return res.json({});
};
"#;
        let syms = parse_and_extract(source, Language::JavaScript);
        let exported: Vec<_> = syms.iter().filter(|s| s.is_exported).collect();
        assert_eq!(
            exported.len(),
            3,
            "expected 3 CommonJS exports, got {:?}",
            exported
                .iter()
                .map(|s| format!("{}:{:?}", s.name, s.kind))
                .collect::<Vec<_>>()
        );

        let create = exported
            .iter()
            .find(|s| s.name == "createComment")
            .expect("createComment should exist");
        assert!(
            matches!(
                create.kind,
                crate::models::SymbolKind::Function | crate::models::SymbolKind::ArrowFunction
            ),
            "createComment should be Function/ArrowFunction kind, got {:?}",
            create.kind
        );

        let update = exported
            .iter()
            .find(|s| s.name == "updatePost")
            .expect("updatePost should exist");
        assert!(
            matches!(
                update.kind,
                crate::models::SymbolKind::Function | crate::models::SymbolKind::ArrowFunction
            ),
            "updatePost should be Function/ArrowFunction kind, got {:?}",
            update.kind
        );

        let delete = exported
            .iter()
            .find(|s| s.name == "deleteItem")
            .expect("deleteItem should exist");
        assert!(
            matches!(
                delete.kind,
                crate::models::SymbolKind::Function | crate::models::SymbolKind::ArrowFunction
            ),
            "deleteItem should be Function/ArrowFunction kind, got {:?}",
            delete.kind
        );
    }

    #[test]
    fn extract_ts_function_parameters() {
        let source = "function greet(name: string, count?: number, ...rest: any[]) {}";
        let syms = parse_and_extract(source, Language::TypeScript);
        let params: Vec<&SymbolInfo> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .collect();
        let names: Vec<&str> = params.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"name"),
            "expected `name` param, got {:?}",
            names
        );
        assert!(
            names.contains(&"count"),
            "expected `count` param, got {:?}",
            names
        );
        assert!(
            names.contains(&"rest"),
            "expected `rest` param, got {:?}",
            names
        );
    }

    #[test]
    fn extract_ts_arrow_parameters() {
        // Both parenthesized arrows (formal_parameters) and bare-identifier arrows
        // should emit Parameter symbols.
        let source = "const a = (x: number) => x + 1;\nconst b = y => y * 2;";
        let syms = parse_and_extract(source, Language::TypeScript);
        let params: Vec<&str> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            params.contains(&"x"),
            "expected `x` param, got {:?}",
            params
        );
        assert!(
            params.contains(&"y"),
            "expected `y` param, got {:?}",
            params
        );
    }

    #[test]
    fn extract_ts_method_parameters_and_local() {
        // Method params + a local `let` inside the body. The local should already
        // be picked up by the pre-existing lexical_declaration capture.
        let source = "class C { run(a: number, b: number) { let total = a + b; return total; } }";
        let syms = parse_and_extract(source, Language::TypeScript);
        let params: Vec<&str> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .map(|s| s.name.as_str())
            .collect();
        assert!(params.contains(&"a"));
        assert!(params.contains(&"b"));
        let total = syms.iter().find(|s| s.name == "total");
        assert!(total.is_some(), "expected local `total` symbol");
        assert_eq!(total.unwrap().kind, SymbolKind::Variable);
    }

    #[test]
    fn extract_js_function_parameters_and_local() {
        // JS uses formal_parameters with bare identifier / rest_pattern children.
        let source = "function add(a, b, ...rest) { let sum = a + b; var v = 0; return sum; }";
        let syms = parse_and_extract(source, Language::JavaScript);
        let params: Vec<&str> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .map(|s| s.name.as_str())
            .collect();
        assert!(params.contains(&"a"), "params: {:?}", params);
        assert!(params.contains(&"b"), "params: {:?}", params);
        assert!(params.contains(&"rest"), "params: {:?}", params);
        let sum = syms.iter().find(|s| s.name == "sum").expect("local `sum`");
        assert_eq!(sum.kind, SymbolKind::Variable);
        let v = syms.iter().find(|s| s.name == "v").expect("local `v`");
        assert_eq!(v.kind, SymbolKind::Variable);
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
        assert_eq!(syms[0].start_line, 1);
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
        let imports =
            parse_and_extract_imports(r#"import React from "react";"#, Language::TypeScript);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "default");
        assert_eq!(imports[0].local_name, "React");
        assert_eq!(imports[0].module_specifier, "react");
        assert!(imports[0].is_external); // bare specifier = external
    }

    #[test]
    fn namespace_import() {
        let imports =
            parse_and_extract_imports(r#"import * as path from "path";"#, Language::TypeScript);
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
        let imports = parse_and_extract_imports(r#"import "./polyfill";"#, Language::TypeScript);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "*");
        assert_eq!(imports[0].local_name, "*");
        assert_eq!(imports[0].module_specifier, "./polyfill");
    }

    #[test]
    fn dynamic_import() {
        let imports =
            parse_and_extract_imports(r#"const mod = import("./lazy");"#, Language::TypeScript);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, "dynamic");
        assert_eq!(imports[0].module_specifier, "./lazy");
    }

    #[test]
    fn reexport_star() {
        let imports = parse_and_extract_imports(r#"export * from "./base";"#, Language::TypeScript);
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
        assert_eq!(imports[0].line, 2);
    }

    // ── resolve_import tests ──

    fn known(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn resolve_extensionless_specifier_to_ts() {
        let files = known(&["src/foo.ts", "src/bar.ts"]);
        let got = resolve_import("src/bar.ts", "./foo", &files);
        assert_eq!(got.as_deref(), Some("src/foo.ts"));
    }

    #[test]
    fn resolve_nodenext_js_specifier_to_ts() {
        let files = known(&["src/foo.ts", "src/bar.ts"]);
        let got = resolve_import("src/bar.ts", "./foo.js", &files);
        assert_eq!(got.as_deref(), Some("src/foo.ts"));
    }

    #[test]
    fn resolve_nodenext_jsx_specifier_to_tsx() {
        let files = known(&["src/Foo.tsx", "src/Bar.tsx"]);
        let got = resolve_import("src/Bar.tsx", "./Foo.jsx", &files);
        assert_eq!(got.as_deref(), Some("src/Foo.tsx"));
    }

    #[test]
    fn resolve_nodenext_mjs_specifier_to_mts() {
        let files = known(&["src/foo.mts", "src/bar.mts"]);
        let got = resolve_import("src/bar.mts", "./foo.mjs", &files);
        assert_eq!(got.as_deref(), Some("src/foo.mts"));
    }

    #[test]
    fn resolve_js_specifier_keeps_real_js_file() {
        // If the .js file actually exists, exact match wins over the .ts swap.
        let files = known(&["src/foo.js", "src/foo.ts", "src/bar.ts"]);
        let got = resolve_import("src/bar.ts", "./foo.js", &files);
        assert_eq!(got.as_deref(), Some("src/foo.js"));
    }

    #[test]
    fn resolve_unresolvable_returns_none() {
        let files = known(&["src/bar.ts"]);
        assert!(resolve_import("src/bar.ts", "./missing.js", &files).is_none());
    }
}
