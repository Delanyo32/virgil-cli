//! Issue #16 C++ references emitter — minimal MVP.

use tree_sitter::Tree;

use crate::languages::{LangRefs, emit_minimal_references};
use crate::models::{ReferencesBucket, SymbolInfo};

const CFG: LangRefs = LangRefs {
    function_scope_kinds: &["function_definition", "lambda_expression"],
    class_scope_kinds: &[
        "class_specifier",
        "struct_specifier",
        "union_specifier",
        "namespace_definition",
    ],
    block_scope_kinds: &["compound_statement"],
    identifier_kinds: &["identifier", "type_identifier", "field_identifier"],
    type_identifier_kinds: &["type_identifier"],
    declaration_parents: &[
        "function_definition",
        "declaration",
        "parameter_declaration",
        "field_declaration",
        "class_specifier",
        "struct_specifier",
        "namespace_definition",
        "template_declaration",
        "init_declarator",
        "function_declarator",
    ],
    call_parents: &["call_expression"],
    assignment_parents: &["assignment_expression"],
    import_parents: &["preproc_include", "using_declaration", "alias_declaration"],
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    emit_minimal_references(tree, source, file_path, symbols, &CFG)
}
