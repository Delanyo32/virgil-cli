//! Issue #16 Go references emitter — minimal MVP.

use tree_sitter::Tree;

use crate::languages::{LangRefs, emit_minimal_references};
use crate::models::{ReferencesBucket, SymbolInfo};

const CFG: LangRefs = LangRefs {
    function_scope_kinds: &[
        "function_declaration",
        "method_declaration",
        "func_literal",
    ],
    class_scope_kinds: &["type_declaration"],
    block_scope_kinds: &["block"],
    identifier_kinds: &["identifier", "type_identifier", "field_identifier"],
    type_identifier_kinds: &["type_identifier"],
    declaration_parents: &[
        "function_declaration",
        "method_declaration",
        "type_spec",
        "var_spec",
        "const_spec",
        "parameter_declaration",
        "short_var_declaration",
    ],
    call_parents: &["call_expression"],
    assignment_parents: &["assignment_statement"],
    import_parents: &["import_declaration", "import_spec"],
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    emit_minimal_references(tree, source, file_path, symbols, &CFG)
}
