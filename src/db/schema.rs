//! DuckDB DDL for the virgil fact store.
//!
//! Stringly composite IDs per ADR-0002. Composite-key tables get an
//! explicit `PRIMARY KEY (...)`; list-valued columns are `VARCHAR[]`.

/// `CREATE TABLE` statements applied in order on a fresh store.
pub fn create_statements() -> &'static [&'static str] {
    &[
        // ─── files & symbols ───────────────────────────────────────────────
        "CREATE TABLE file (\
            path VARCHAR PRIMARY KEY, \
            language VARCHAR NOT NULL, \
            repo_id VARCHAR NOT NULL, \
            content VARCHAR NOT NULL\
         )",
        "CREATE TABLE symbol (\
            id VARCHAR PRIMARY KEY, \
            kind VARCHAR NOT NULL, \
            name VARCHAR NOT NULL, \
            qualified_name VARCHAR NOT NULL, \
            language VARCHAR NOT NULL, \
            visibility VARCHAR NOT NULL, \
            file_path VARCHAR NOT NULL, \
            parent_id VARCHAR, \
            is_async BOOLEAN NOT NULL, \
            is_static BOOLEAN NOT NULL, \
            is_abstract BOOLEAN NOT NULL, \
            is_mutable BOOLEAN NOT NULL, \
            exported BOOLEAN NOT NULL\
         )",
        // span: positional metadata per entity. entity_id is a
        // symbol/comment/call-site id.
        "CREATE TABLE span (\
            entity_id VARCHAR NOT NULL, \
            file_path VARCHAR NOT NULL, \
            start_byte BIGINT NOT NULL, \
            end_byte BIGINT NOT NULL, \
            start_line BIGINT NOT NULL, \
            end_line BIGINT NOT NULL, \
            start_col BIGINT NOT NULL, \
            end_col BIGINT NOT NULL, \
            PRIMARY KEY (entity_id, file_path)\
         )",
        // ─── graph edges ───────────────────────────────────────────────────
        // Raw call sites — one row per call expression. caller_id is
        // NULL for top-level calls. Cross-file resolution to a symbol
        // id is done by `resolve_and_emit_call_edges` at build end.
        "CREATE TABLE call_site (\
            id VARCHAR PRIMARY KEY, \
            caller_id VARCHAR, \
            callee_name VARCHAR NOT NULL, \
            receiver VARCHAR, \
            file_path VARCHAR NOT NULL, \
            start_byte BIGINT NOT NULL, \
            end_byte BIGINT NOT NULL\
         )",
        // Resolved call edges, materialised at build time.
        "CREATE TABLE call_edge (\
            caller_id VARCHAR NOT NULL, \
            callee_id VARCHAR NOT NULL, \
            file_path VARCHAR NOT NULL, \
            PRIMARY KEY (caller_id, callee_id)\
         )",
        // ─── ADR-0005 fact-emission relations ──────────────────────────────
        "CREATE TABLE occurrence (\
            id VARCHAR PRIMARY KEY, \
            name VARCHAR NOT NULL, \
            file_path VARCHAR NOT NULL, \
            start_byte BIGINT NOT NULL, \
            end_byte BIGINT NOT NULL, \
            enclosing_symbol_id VARCHAR, \
            enclosing_scope_id VARCHAR NOT NULL, \
            occurrence_kind VARCHAR NOT NULL\
         )",
        // parent_id is NULL for the file/module scope.
        "CREATE TABLE scope (\
            id VARCHAR PRIMARY KEY, \
            parent_id VARCHAR, \
            file_path VARCHAR NOT NULL, \
            kind VARCHAR NOT NULL, \
            start_byte BIGINT NOT NULL, \
            end_byte BIGINT NOT NULL\
         )",
        // name → symbol_id within a scope. Shadowing permitted.
        "CREATE TABLE binding (\
            scope_id VARCHAR NOT NULL, \
            name VARCHAR NOT NULL, \
            start_byte BIGINT NOT NULL, \
            symbol_id VARCHAR, \
            binding_kind VARCHAR NOT NULL, \
            PRIMARY KEY (scope_id, name, start_byte)\
         )",
        "CREATE TABLE extends (\
            child_id VARCHAR NOT NULL, \
            parent_id VARCHAR NOT NULL, \
            PRIMARY KEY (child_id, parent_id)\
         )",
        "CREATE TABLE implements (\
            impl_id VARCHAR NOT NULL, \
            interface_id VARCHAR NOT NULL, \
            PRIMARY KEY (impl_id, interface_id)\
         )",
        "CREATE TABLE imports (\
            importer_file_id VARCHAR NOT NULL, \
            imported_id VARCHAR NOT NULL, \
            PRIMARY KEY (importer_file_id, imported_id)\
         )",
        // raw imports (pre-resolution), preserved per file.
        "CREATE TABLE raw_import (\
            file_path VARCHAR NOT NULL, \
            position BIGINT NOT NULL, \
            raw_path VARCHAR NOT NULL, \
            language VARCHAR NOT NULL, \
            kind VARCHAR NOT NULL, \
            PRIMARY KEY (file_path, position)\
         )",
        // ─── signatures & types ────────────────────────────────────────────
        "CREATE TABLE parameter (\
            id VARCHAR PRIMARY KEY, \
            name VARCHAR NOT NULL, \
            function_id VARCHAR NOT NULL, \
            position BIGINT NOT NULL, \
            type_id VARCHAR, \
            is_optional BOOLEAN NOT NULL, \
            has_default BOOLEAN NOT NULL, \
            is_taint_source BOOLEAN NOT NULL\
         )",
        "CREATE TABLE returns_type (\
            function_id VARCHAR PRIMARY KEY, \
            type_id VARCHAR NOT NULL\
         )",
        "CREATE TABLE throws (\
            function_id VARCHAR NOT NULL, \
            exception_type_id VARCHAR NOT NULL, \
            PRIMARY KEY (function_id, exception_type_id)\
         )",
        "CREATE TABLE field_type (\
            symbol_id VARCHAR PRIMARY KEY, \
            type_id VARCHAR NOT NULL\
         )",
        // local variable -> declared/inferred type name (cheap-declaration
        // cases only: `Foo x = ...`, `var x = new Foo()`, `x := NewFoo()`,
        // `let x = Foo::new()`). File-scoped (start_byte disambiguates
        // shadowing). type_name is the bare class/struct name so the call
        // resolver can join it to symbol.name directly.
        "CREATE TABLE local_type (\
            file_path VARCHAR NOT NULL, \
            name VARCHAR NOT NULL, \
            type_name VARCHAR NOT NULL, \
            start_byte BIGINT NOT NULL, \
            PRIMARY KEY (file_path, name, start_byte)\
         )",
        "CREATE TABLE type (\
            id VARCHAR PRIMARY KEY, \
            kind VARCHAR NOT NULL, \
            language VARCHAR NOT NULL, \
            display_name VARCHAR NOT NULL, \
            canonical_name VARCHAR\
         )",
        // ─── comments ──────────────────────────────────────────────────────
        "CREATE TABLE comment (\
            id VARCHAR PRIMARY KEY, \
            documents_id VARCHAR, \
            file_path VARCHAR NOT NULL, \
            kind VARCHAR NOT NULL, \
            is_doc BOOLEAN NOT NULL, \
            text VARCHAR NOT NULL, \
            todo_kind VARCHAR, \
            start_byte BIGINT NOT NULL, \
            end_byte BIGINT NOT NULL\
         )",
        // ─── per-language attribute tables (populated lazily by language) ──
        "CREATE TABLE rust_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            is_unsafe BOOLEAN NOT NULL, \
            is_const BOOLEAN NOT NULL, \
            derives VARCHAR[] NOT NULL\
         )",
        "CREATE TABLE python_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            decorators VARCHAR[] NOT NULL, \
            is_generator BOOLEAN NOT NULL, \
            is_coroutine BOOLEAN NOT NULL, \
            docstring_style VARCHAR\
         )",
        "CREATE TABLE typescript_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            is_readonly BOOLEAN NOT NULL, \
            is_optional BOOLEAN NOT NULL, \
            type_parameters VARCHAR[] NOT NULL\
         )",
        "CREATE TABLE cpp_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            is_virtual BOOLEAN NOT NULL, \
            is_const BOOLEAN NOT NULL, \
            is_noexcept BOOLEAN NOT NULL, \
            is_template BOOLEAN NOT NULL, \
            is_constexpr BOOLEAN NOT NULL, \
            is_override BOOLEAN NOT NULL, \
            is_final BOOLEAN NOT NULL\
         )",
        "CREATE TABLE csharp_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            attributes VARCHAR[] NOT NULL, \
            is_partial BOOLEAN NOT NULL, \
            is_sealed BOOLEAN NOT NULL\
         )",
        "CREATE TABLE go_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            is_exported BOOLEAN NOT NULL, \
            has_receiver BOOLEAN NOT NULL, \
            build_tags VARCHAR[] NOT NULL\
         )",
        "CREATE TABLE php_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            is_final BOOLEAN NOT NULL, \
            uses_traits VARCHAR[] NOT NULL, \
            attributes VARCHAR[] NOT NULL\
         )",
        "CREATE TABLE c_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            is_file_static BOOLEAN NOT NULL, \
            is_extern BOOLEAN NOT NULL, \
            is_inline BOOLEAN NOT NULL, \
            is_const BOOLEAN NOT NULL, \
            is_volatile BOOLEAN NOT NULL, \
            is_restrict BOOLEAN NOT NULL, \
            gcc_attributes VARCHAR[] NOT NULL\
         )",
        "CREATE TABLE java_attrs (\
            symbol_id VARCHAR PRIMARY KEY, \
            annotations VARCHAR[] NOT NULL, \
            is_final BOOLEAN NOT NULL, \
            is_synchronized BOOLEAN NOT NULL, \
            throws_clause VARCHAR[] NOT NULL\
         )",
        // ─── staging tables (parse-time, dropped after resolve) ───────────
        // Inheritance is the one extractor output that needs cross-file
        // symbol-id resolution. Workers write rows here during absorb;
        // a SQL `INSERT...SELECT` joins against `symbol` + `imports`
        // afterwards to populate `extends` / `implements`.
        "CREATE TABLE raw_inheritance (\
            file_path VARCHAR NOT NULL, \
            child_name VARCHAR NOT NULL, \
            child_kind VARCHAR NOT NULL, \
            child_start_line BIGINT NOT NULL, \
            child_start_col BIGINT NOT NULL, \
            parent_leaf VARCHAR NOT NULL, \
            parent_canonical VARCHAR, \
            kind VARCHAR NOT NULL\
         )",
        // ─── derived facts ─────────────────────────────────────────────────
        "CREATE TABLE file_classification (\
            path VARCHAR PRIMARY KEY, \
            is_test BOOLEAN NOT NULL, \
            is_barrel BOOLEAN NOT NULL, \
            is_generated BOOLEAN NOT NULL\
         )",
        // ─── metadata ──────────────────────────────────────────────────────
        "CREATE TABLE build_meta (\
            key VARCHAR PRIMARY KEY, \
            value VARCHAR NOT NULL\
         )",
    ]
}

/// Secondary indices, applied after [`create_statements`].
pub fn index_statements() -> &'static [&'static str] {
    &[
        "CREATE INDEX idx_symbol_by_name ON symbol(name)",
        "CREATE INDEX idx_symbol_by_file ON symbol(file_path)",
        "CREATE INDEX idx_imports_by_imported ON imports(imported_id)",
        "CREATE INDEX idx_imports_by_importer ON imports(importer_file_id)",
        "CREATE INDEX idx_comment_by_file ON comment(file_path)",
        "CREATE INDEX idx_comment_by_documents ON comment(documents_id)",
        "CREATE INDEX idx_occurrence_by_name ON occurrence(name)",
        "CREATE INDEX idx_binding_by_name ON binding(name)",
        "CREATE INDEX idx_scope_by_file ON scope(file_path)",
        "CREATE INDEX idx_call_site_by_caller ON call_site(caller_id)",
        "CREATE INDEX idx_call_site_by_name ON call_site(callee_name)",
    ]
}
