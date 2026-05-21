//! Issue #16 PHP references emitter — minimal MVP.

use tree_sitter::Tree;

use crate::languages::{LangRefs, emit_minimal_references};
use crate::models::{ReferencesBucket, SymbolInfo};

const CFG: LangRefs = LangRefs {
    function_scope_kinds: &[
        "function_definition",
        "method_declaration",
        "anonymous_function_creation_expression",
        "arrow_function",
    ],
    class_scope_kinds: &[
        "class_declaration",
        "interface_declaration",
        "trait_declaration",
    ],
    block_scope_kinds: &["compound_statement"],
    identifier_kinds: &["name", "variable_name"],
    type_identifier_kinds: &[],
    declaration_parents: &[
        "function_definition",
        "method_declaration",
        "class_declaration",
        "interface_declaration",
        "trait_declaration",
        "property_declaration",
        "property_element",
        "simple_parameter",
        "variadic_parameter",
    ],
    call_parents: &[
        "function_call_expression",
        "member_call_expression",
        "scoped_call_expression",
    ],
    assignment_parents: &["assignment_expression"],
    import_parents: &["namespace_use_declaration", "namespace_use_clause"],
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    emit_minimal_references(tree, source, file_path, symbols, &CFG)
}
