//! Cozo relation schema for the Datalog-model migration.
//!
//! Defined in `docs/virgil-datalog-schema.md`. The shapes here are the
//! authority — any divergence between this file and the doc is a bug.
//!
//! Phase 1 (this file) lands the relations with String IDs, the new
//! `references` shape (match_index key, nullable `referent_id`), and the
//! `field_type` relation. Per-language `*_attrs` tables are declared empty;
//! they get populated in Phase 4 (issue #15).

/// All `:create` statements, applied in order when a fresh store is opened.
pub fn create_statements() -> &'static [&'static str] {
    &[
        // ─── files & symbols ───────────────────────────────────────────────
        ":create file {path: String => language: String, repo_id: String}",
        ":create symbol {id: String => \
            kind: String, name: String, qualified_name: String, language: String, \
            visibility: String, file_path: String, parent_id: String?, \
            is_async: Bool, is_static: Bool, is_abstract: Bool, is_mutable: Bool, \
            exported: Bool}",
        // span: positional metadata per entity. entity_id refers to a
        // symbol/comment/call-site id.
        ":create span {entity_id: String, file_path: String => \
            start_byte: Int, end_byte: Int, \
            start_line: Int, end_line: Int, \
            start_col: Int, end_col: Int}",
        // ─── graph edges ───────────────────────────────────────────────────
        ":create calls {caller_id: String, callee_id: String => \
            call_site_file: String, call_site_start_byte: Int, \
            call_site_end_byte: Int, is_direct: Bool}",
        ":create references {referrer_id: String, site_file: String, \
            site_start_byte: Int, match_index: Int => \
            referent_id: String?, ref_kind: String}",
        ":create extends {child_id: String, parent_id: String}",
        ":create implements {impl_id: String, interface_id: String}",
        ":create imports {importer_file_id: String, imported_id: String}",
        // raw imports (pre-resolution), preserved per file for incremental refresh
        ":create raw_import {file_path: String, position: Int => \
            raw_path: String, language: String, kind: String}",
        // ─── signatures & types ────────────────────────────────────────────
        ":create parameter {id: String => \
            name: String, function_id: String, position: Int, \
            type_id: String?, is_optional: Bool, has_default: Bool, \
            is_taint_source: Bool}",
        ":create returns_type {function_id: String => type_id: String}",
        ":create throws {function_id: String, exception_type_id: String}",
        ":create field_type {symbol_id: String => type_id: String}",
        ":create type {id: String => \
            kind: String, language: String, \
            display_name: String, canonical_name: String?}",
        // ─── comments ──────────────────────────────────────────────────────
        ":create comment {id: String => \
            documents_id: String?, file_path: String, kind: String, \
            is_doc: Bool, text: String, todo_kind: String?, \
            start_byte: Int, end_byte: Int}",
        // ─── per-language attribute tables (declared, empty until Phase 4) ─
        ":create rust_attrs {symbol_id: String => \
            is_unsafe: Bool, is_const: Bool, derives: [String]}",
        ":create python_attrs {symbol_id: String => \
            decorators: [String], is_generator: Bool, is_coroutine: Bool, \
            docstring_style: String?}",
        ":create typescript_attrs {symbol_id: String => \
            is_readonly: Bool, is_optional: Bool, type_parameters: [String]}",
        ":create cpp_attrs {symbol_id: String => \
            is_virtual: Bool, is_const: Bool, is_noexcept: Bool, \
            is_template: Bool, is_constexpr: Bool, is_override: Bool, \
            is_final: Bool}",
        ":create csharp_attrs {symbol_id: String => \
            attributes: [String], is_partial: Bool, is_sealed: Bool}",
        ":create go_attrs {symbol_id: String => \
            is_exported: Bool, has_receiver: Bool, build_tags: [String]}",
        ":create php_attrs {symbol_id: String => \
            is_final: Bool, uses_traits: [String], attributes: [String]}",
        ":create c_attrs {symbol_id: String => \
            is_file_static: Bool, is_extern: Bool, is_inline: Bool, \
            is_const: Bool, is_volatile: Bool, is_restrict: Bool, \
            gcc_attributes: [String]}",
        ":create java_attrs {symbol_id: String => \
            annotations: [String], is_final: Bool, is_synchronized: Bool, \
            throws_clause: [String]}",
        // ─── derived facts (carryover from old schema) ─────────────────────
        ":create file_classification {path: String => \
            is_test: Bool, is_barrel: Bool, is_generated: Bool}",
        ":create nolint {file_path: String, line: Int => suppressed_pattern: String}",
        // ─── metadata ──────────────────────────────────────────────────────
        ":create build_meta {key: String => value: String}",
        ":create build_meta_files {file_path: String => \
            hash: String, size: Int, mtime: Int}",
    ]
}

/// Indices applied after relations exist.
pub fn index_statements() -> &'static [&'static str] {
    &[
        "::index create symbol:by_name {name}",
        "::index create symbol:by_qname {qualified_name}",
        "::index create symbol:by_file {file_path}",
        "::index create calls:by_callee {callee_id}",
        "::index create references:by_referent {referent_id}",
        "::index create imports:by_imported {imported_id}",
        "::index create comment:by_file {file_path}",
        "::index create comment:by_documents {documents_id}",
    ]
}
