//! Schema definitions for the Cozo fact store.
//!
//! Issue 02 scope: cross-function graph relations + indices + metadata
//! relations. CFG relations land in issue 03, metric relations in issue 04.

/// Cross-function graph + metadata `:create` statements. Applied in order
/// when a fresh store is opened.
pub fn create_statements() -> &'static [&'static str] {
    &[
        // ---- Cross-function graph ----
        ":create file {path: String => language: String}",
        ":create symbol {id: Int => name: String, kind: String, \
            file_path: String, start_line: Int, end_line: Int, exported: Bool}",
        ":create callsite {id: Int => name: String, file_path: String, \
            line: Int, caller_symbol_id: Int?, enclosing_test_name: String?}",
        ":create call_arg {callsite_id: Int, position: Int => value: String}",
        ":create parameter {id: Int => name: String, function_id: Int, \
            position: Int, is_taint_source: Bool}",
        ":create external_source {id: Int => kind: String, file_path: String, line: Int}",
        ":create edge_defined_in {symbol_id: Int, file_path: String}",
        ":create edge_calls {caller_id: Int, callee_id: Int}",
        ":create edge_imports {from_path: String, to_path: String}",
        ":create edge_exports {file_path: String, symbol_id: Int}",
        ":create edge_contains {parent_id: Int, child_id: Int}",
        // ---- Metadata ----
        ":create build_meta {key: String => value: String}",
        ":create build_meta_files {file_path: String => \
            hash: String, size: Int, mtime: Int}",
    ]
}

/// Index `::index create` statements. Applied after relations exist.
pub fn index_statements() -> &'static [&'static str] {
    &[
        "::index create symbol:by_name {name}",
        "::index create symbol:by_file_line {file_path, start_line}",
        "::index create callsite:by_name {name}",
        "::index create callsite:by_file {file_path}",
        "::index create edge_calls:by_callee {callee_id}",
        "::index create edge_imports:by_to {to_path}",
    ]
}
