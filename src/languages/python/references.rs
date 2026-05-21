//! Issue #16 Python references emitter — minimal MVP via
//! `emit_minimal_references`.

use tree_sitter::Tree;

use crate::languages::{LangRefs, emit_minimal_references};
use crate::models::{ReferencesBucket, SymbolInfo};

const CFG: LangRefs = LangRefs {
    function_scope_kinds: &["function_definition", "lambda"],
    class_scope_kinds: &["class_definition"],
    block_scope_kinds: &["block"],
    identifier_kinds: &["identifier"],
    type_identifier_kinds: &[],
    declaration_parents: &[
        "function_definition",
        "class_definition",
        "parameters",
        "typed_parameter",
        "typed_default_parameter",
        "default_parameter",
        "assignment",
    ],
    call_parents: &["call"],
    assignment_parents: &["assignment", "augmented_assignment"],
    import_parents: &[
        "import_statement",
        "import_from_statement",
        "aliased_import",
        "dotted_name",
    ],
};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    emit_minimal_references(tree, source, file_path, symbols, &CFG)
}
