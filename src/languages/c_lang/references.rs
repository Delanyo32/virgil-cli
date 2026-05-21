//! Issue #16 C references emitter — minimal MVP.

use tree_sitter::Tree;

use crate::languages::{LangRefs, emit_minimal_references};
use crate::models::{ReferencesBucket, SymbolInfo};

const CFG: LangRefs = LangRefs {
    function_scope_kinds: &["function_definition"],
    class_scope_kinds: &[],
    block_scope_kinds: &["compound_statement"],
    identifier_kinds: &["identifier", "type_identifier", "field_identifier"],
    type_identifier_kinds: &["type_identifier"],
    declaration_parents: &[
        "function_definition",
        "declaration",
        "parameter_declaration",
        "field_declaration",
        "type_definition",
        "init_declarator",
        "function_declarator",
    ],
    call_parents: &["call_expression"],
    assignment_parents: &["assignment_expression"],
    import_parents: &["preproc_include"],
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    emit_minimal_references(tree, source, file_path, symbols, &CFG)
}
