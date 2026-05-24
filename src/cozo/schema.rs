//! Cozo relation schema for the Datalog-model migration.
//!
//! Defined in `docs/virgil-datalog-schema.md`. The shapes here are the
//! authority — any divergence between this file and the doc is a bug.
//!
//! Phase 1 (this file) lands the relations with String IDs and the
//! `field_type` relation. Per-language `*_attrs` tables are declared
//! empty; they get populated in Phase 4 (issue #15).
//!
//! The eager Cozoscript reference resolver and its materialised
//! `references` relation were removed (schema v6) — callers that need
//! resolved references run their own Cozoscript over the raw
//! `occurrence`/`scope`/`binding` facts at query time.

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
        // ─── ADR-0005 fact-emission relations (issue #16) ──────────────────
        // Raw facts emitted by per-language extractors. Callers that
        // want resolved references join occurrence × scope × binding ×
        // imports themselves in Cozoscript at query time.
        ":create occurrence {id: String => \
            name: String, file_path: String, \
            start_byte: Int, end_byte: Int, \
            enclosing_symbol_id: String?, enclosing_scope_id: String, \
            occurrence_kind: String}",
        // Lexical scope chain per file. `parent_id` is null for the
        // file/module scope.
        ":create scope {id: String => \
            parent_id: String?, file_path: String, kind: String, \
            start_byte: Int, end_byte: Int}",
        // name → symbol_id binding within a specific scope. Shadowing
        // is permitted; downstream resolution picks by `start_byte` order.
        ":create binding {scope_id: String, name: String, start_byte: Int => \
            symbol_id: String?, binding_kind: String}",
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
        "::index create imports:by_imported {imported_id}",
        "::index create imports:by_importer {importer_file_id}",
        "::index create comment:by_file {file_path}",
        "::index create comment:by_documents {documents_id}",
        // Issue #16 — indices for callers writing their own resolution
        // Cozoscript against the raw occurrence/scope/binding facts.
        "::index create occurrence:by_name {name}",
        "::index create binding:by_name {name}",
        "::index create scope:by_file {file_path}",
    ]
}
