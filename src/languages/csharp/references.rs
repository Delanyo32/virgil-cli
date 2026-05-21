//! Issue #16 C# references emitter — minimal MVP.

use tree_sitter::Tree;

use crate::languages::{LangRefs, emit_minimal_references};
use crate::models::{ReferencesBucket, SymbolInfo};

const CFG: LangRefs = LangRefs {
    function_scope_kinds: &[
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
        "local_function_statement",
    ],
    class_scope_kinds: &[
        "class_declaration",
        "interface_declaration",
        "struct_declaration",
        "record_declaration",
        "namespace_declaration",
        "file_scoped_namespace_declaration",
    ],
    block_scope_kinds: &["block"],
    identifier_kinds: &["identifier"],
    type_identifier_kinds: &[],
    declaration_parents: &[
        "method_declaration",
        "constructor_declaration",
        "class_declaration",
        "interface_declaration",
        "struct_declaration",
        "record_declaration",
        "parameter",
        "variable_declarator",
        "field_declaration",
        "property_declaration",
        "namespace_declaration",
    ],
    call_parents: &["invocation_expression", "object_creation_expression"],
    assignment_parents: &["assignment_expression"],
    import_parents: &["using_directive"],
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    emit_minimal_references(tree, source, file_path, symbols, &CFG)
}
