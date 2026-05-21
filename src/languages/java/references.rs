//! Issue #16 Java references emitter — minimal MVP.

use tree_sitter::Tree;

use crate::languages::{LangRefs, emit_minimal_references};
use crate::models::{ReferencesBucket, SymbolInfo};

const CFG: LangRefs = LangRefs {
    function_scope_kinds: &[
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
    ],
    class_scope_kinds: &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "record_declaration",
    ],
    block_scope_kinds: &["block"],
    identifier_kinds: &["identifier", "type_identifier"],
    type_identifier_kinds: &["type_identifier"],
    declaration_parents: &[
        "method_declaration",
        "constructor_declaration",
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "record_declaration",
        "formal_parameter",
        "variable_declarator",
        "field_declaration",
        "local_variable_declaration",
    ],
    call_parents: &["method_invocation", "object_creation_expression"],
    assignment_parents: &["assignment_expression"],
    import_parents: &["import_declaration", "scoped_identifier"],
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    emit_minimal_references(tree, source, file_path, symbols, &CFG)
}
