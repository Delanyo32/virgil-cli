//! Batched row writer fed by the graph absorber.
//!
//! 1:1 port of `src/cozo/writer.rs` — same `push_*` method shapes, same
//! accumulate-then-flush model. Each table has its own `Vec<Vec<Value>>`
//! buffer. `flush()` opens a DuckDB `Appender` per non-empty table and
//! streams the rows in.
//!
//! Appender vs hand-rolled Arrow batches: question 4's locked answer
//! was Arrow, but during implementation Appender turned out to be the
//! direct mechanical port (~5× less code, same internal columnar batch
//! path inside duckdb). Documented in
//! `docs/experiments/duckdb-swap.md` under "Deviations".

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use duckdb::Connection;
use duckdb::types::Value;

use super::store::DbStore;

#[allow(dead_code)]
const FLUSH_BATCH: usize = 10_000;

type Row = Vec<Value>;

/// Accumulates per-relation rows and flushes them to a [`DbStore`].
#[derive(Default)]
pub struct DbWriter {
    file: Vec<Row>,
    symbol: Vec<Row>,
    span: Vec<Row>,
    calls: Vec<Row>,
    call_site: Vec<Row>,
    call_edge: Vec<Row>,
    extends: Vec<Row>,
    implements: Vec<Row>,
    raw_inheritance: Vec<Row>,
    imports: Vec<Row>,
    raw_import: Vec<Row>,
    parameter: Vec<Row>,
    returns_type: Vec<Row>,
    throws: Vec<Row>,
    field_type: Vec<Row>,
    ty: Vec<Row>,
    comment: Vec<Row>,
    file_classification: Vec<Row>,
    nolint: Vec<Row>,
    build_meta: Vec<Row>,
    build_meta_files: Vec<Row>,
    occurrence: Vec<Row>,
    scope: Vec<Row>,
    binding: Vec<Row>,
    local_type: Vec<Row>,
    rust_attrs: Vec<Row>,
    python_attrs: Vec<Row>,
    typescript_attrs: Vec<Row>,
    cpp_attrs: Vec<Row>,
    csharp_attrs: Vec<Row>,
    go_attrs: Vec<Row>,
    php_attrs: Vec<Row>,
    c_attrs: Vec<Row>,
    java_attrs: Vec<Row>,
}

impl DbWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append every row from `other` into `self`, leaving `other` empty.
    pub fn merge(&mut self, other: &mut DbWriter) {
        self.file.append(&mut other.file);
        self.symbol.append(&mut other.symbol);
        self.span.append(&mut other.span);
        self.calls.append(&mut other.calls);
        self.call_site.append(&mut other.call_site);
        self.call_edge.append(&mut other.call_edge);
        self.extends.append(&mut other.extends);
        self.implements.append(&mut other.implements);
        self.raw_inheritance.append(&mut other.raw_inheritance);
        self.imports.append(&mut other.imports);
        self.raw_import.append(&mut other.raw_import);
        self.parameter.append(&mut other.parameter);
        self.returns_type.append(&mut other.returns_type);
        self.throws.append(&mut other.throws);
        self.field_type.append(&mut other.field_type);
        self.ty.append(&mut other.ty);
        self.comment.append(&mut other.comment);
        self.file_classification
            .append(&mut other.file_classification);
        self.nolint.append(&mut other.nolint);
        self.build_meta.append(&mut other.build_meta);
        self.build_meta_files.append(&mut other.build_meta_files);
        self.occurrence.append(&mut other.occurrence);
        self.scope.append(&mut other.scope);
        self.binding.append(&mut other.binding);
        self.local_type.append(&mut other.local_type);
        self.rust_attrs.append(&mut other.rust_attrs);
        self.python_attrs.append(&mut other.python_attrs);
        self.typescript_attrs.append(&mut other.typescript_attrs);
        self.cpp_attrs.append(&mut other.cpp_attrs);
        self.csharp_attrs.append(&mut other.csharp_attrs);
        self.go_attrs.append(&mut other.go_attrs);
        self.php_attrs.append(&mut other.php_attrs);
        self.c_attrs.append(&mut other.c_attrs);
        self.java_attrs.append(&mut other.java_attrs);
    }

    pub fn push_file(&mut self, path: &str, language: &str, repo_id: &str) {
        self.file
            .push(vec![text(path), text(language), text(repo_id)]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_symbol(
        &mut self,
        id: &str,
        kind: &str,
        name: &str,
        qualified_name: &str,
        language: &str,
        visibility: &str,
        file_path: &str,
        parent_id: Option<&str>,
        is_async: bool,
        is_static: bool,
        is_abstract: bool,
        is_mutable: bool,
        exported: bool,
    ) {
        self.symbol.push(vec![
            text(id),
            text(kind),
            text(name),
            text(qualified_name),
            text(language),
            text(visibility),
            text(file_path),
            opt_text(parent_id),
            Value::Boolean(is_async),
            Value::Boolean(is_static),
            Value::Boolean(is_abstract),
            Value::Boolean(is_mutable),
            Value::Boolean(exported),
        ]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_span(
        &mut self,
        entity_id: &str,
        file_path: &str,
        start_byte: i64,
        end_byte: i64,
        start_line: i64,
        end_line: i64,
        start_col: i64,
        end_col: i64,
    ) {
        self.span.push(vec![
            text(entity_id),
            text(file_path),
            big(start_byte),
            big(end_byte),
            big(start_line),
            big(end_line),
            big(start_col),
            big(end_col),
        ]);
    }

    pub fn push_calls(
        &mut self,
        caller_id: &str,
        callee_id: &str,
        call_site_file: &str,
        call_site_start_byte: i64,
        call_site_end_byte: i64,
        is_direct: bool,
    ) {
        self.calls.push(vec![
            text(caller_id),
            text(callee_id),
            text(call_site_file),
            big(call_site_start_byte),
            big(call_site_end_byte),
            Value::Boolean(is_direct),
        ]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_call_site(
        &mut self,
        id: &str,
        caller_id: Option<&str>,
        callee_name: &str,
        receiver: Option<&str>,
        file_path: &str,
        start_byte: i64,
        end_byte: i64,
    ) {
        self.call_site.push(vec![
            text(id),
            opt_text(caller_id),
            text(callee_name),
            opt_text(receiver),
            text(file_path),
            big(start_byte),
            big(end_byte),
        ]);
    }

    pub fn push_call_edge(&mut self, caller_id: &str, callee_id: &str, file_path: &str) {
        self.call_edge
            .push(vec![text(caller_id), text(callee_id), text(file_path)]);
    }

    pub fn push_extends(&mut self, child_id: &str, parent_id: &str) {
        self.extends.push(vec![text(child_id), text(parent_id)]);
    }

    pub fn push_implements(&mut self, impl_id: &str, interface_id: &str) {
        self.implements
            .push(vec![text(impl_id), text(interface_id)]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_raw_inheritance(
        &mut self,
        file_path: &str,
        child_name: &str,
        child_kind: &str,
        child_start_line: i64,
        child_start_col: i64,
        parent_leaf: &str,
        parent_canonical: Option<&str>,
        kind: &str,
    ) {
        self.raw_inheritance.push(vec![
            text(file_path),
            text(child_name),
            text(child_kind),
            big(child_start_line),
            big(child_start_col),
            text(parent_leaf),
            match parent_canonical {
                Some(s) => text(s),
                None => Value::Null,
            },
            text(kind),
        ]);
    }

    pub fn push_imports(&mut self, importer_file_id: &str, imported_id: &str) {
        self.imports
            .push(vec![text(importer_file_id), text(imported_id)]);
    }

    pub fn push_raw_import(
        &mut self,
        file_path: &str,
        position: i64,
        raw_path: &str,
        language: &str,
        kind: &str,
    ) {
        self.raw_import.push(vec![
            text(file_path),
            big(position),
            text(raw_path),
            text(language),
            text(kind),
        ]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_parameter(
        &mut self,
        id: &str,
        name: &str,
        function_id: &str,
        position: i64,
        type_id: Option<&str>,
        is_optional: bool,
        has_default: bool,
        is_taint_source: bool,
    ) {
        self.parameter.push(vec![
            text(id),
            text(name),
            text(function_id),
            big(position),
            opt_text(type_id),
            Value::Boolean(is_optional),
            Value::Boolean(has_default),
            Value::Boolean(is_taint_source),
        ]);
    }

    pub fn push_returns_type(&mut self, function_id: &str, type_id: &str) {
        self.returns_type
            .push(vec![text(function_id), text(type_id)]);
    }

    pub fn push_throws(&mut self, function_id: &str, exception_type_id: &str) {
        self.throws
            .push(vec![text(function_id), text(exception_type_id)]);
    }

    pub fn push_field_type(&mut self, symbol_id: &str, type_id: &str) {
        self.field_type.push(vec![text(symbol_id), text(type_id)]);
    }

    pub fn push_type(
        &mut self,
        id: &str,
        kind: &str,
        language: &str,
        display_name: &str,
        canonical_name: Option<&str>,
    ) {
        self.ty.push(vec![
            text(id),
            text(kind),
            text(language),
            text(display_name),
            opt_text(canonical_name),
        ]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_comment(
        &mut self,
        id: &str,
        documents_id: Option<&str>,
        file_path: &str,
        kind: &str,
        is_doc: bool,
        text_body: &str,
        todo_kind: Option<&str>,
        start_byte: i64,
        end_byte: i64,
    ) {
        self.comment.push(vec![
            text(id),
            opt_text(documents_id),
            text(file_path),
            text(kind),
            Value::Boolean(is_doc),
            text(text_body),
            opt_text(todo_kind),
            big(start_byte),
            big(end_byte),
        ]);
    }

    pub fn push_file_classification(
        &mut self,
        path: &str,
        is_test: bool,
        is_barrel: bool,
        is_generated: bool,
    ) {
        self.file_classification.push(vec![
            text(path),
            Value::Boolean(is_test),
            Value::Boolean(is_barrel),
            Value::Boolean(is_generated),
        ]);
    }

    pub fn push_nolint(&mut self, file_path: &str, line: i64, suppressed_pattern: &str) {
        self.nolint
            .push(vec![text(file_path), big(line), text(suppressed_pattern)]);
    }

    pub fn push_build_meta(&mut self, key: &str, value: &str) {
        self.build_meta.push(vec![text(key), text(value)]);
    }

    pub fn push_build_meta_file(&mut self, file_path: &str, hash: &str, size: i64, mtime: i64) {
        self.build_meta_files
            .push(vec![text(file_path), text(hash), big(size), big(mtime)]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_occurrence(
        &mut self,
        id: &str,
        name: &str,
        file_path: &str,
        start_byte: i64,
        end_byte: i64,
        enclosing_symbol_id: Option<&str>,
        enclosing_scope_id: &str,
        occurrence_kind: &str,
    ) {
        self.occurrence.push(vec![
            text(id),
            text(name),
            text(file_path),
            big(start_byte),
            big(end_byte),
            opt_text(enclosing_symbol_id),
            text(enclosing_scope_id),
            text(occurrence_kind),
        ]);
    }

    pub fn push_scope(
        &mut self,
        id: &str,
        parent_id: Option<&str>,
        file_path: &str,
        kind: &str,
        start_byte: i64,
        end_byte: i64,
    ) {
        self.scope.push(vec![
            text(id),
            opt_text(parent_id),
            text(file_path),
            text(kind),
            big(start_byte),
            big(end_byte),
        ]);
    }

    pub fn push_binding(
        &mut self,
        scope_id: &str,
        name: &str,
        start_byte: i64,
        symbol_id: Option<&str>,
        binding_kind: &str,
    ) {
        self.binding.push(vec![
            text(scope_id),
            text(name),
            big(start_byte),
            opt_text(symbol_id),
            text(binding_kind),
        ]);
    }

    pub fn push_local_type(&mut self, file_path: &str, name: &str, type_name: &str, start_byte: i64) {
        self.local_type.push(vec![
            text(file_path),
            text(name),
            text(type_name),
            big(start_byte),
        ]);
    }

    pub fn push_rust_attrs(
        &mut self,
        symbol_id: &str,
        is_unsafe: bool,
        is_const: bool,
        derives: &[String],
    ) {
        self.rust_attrs.push(vec![
            text(symbol_id),
            Value::Boolean(is_unsafe),
            Value::Boolean(is_const),
            list_text(derives),
        ]);
    }

    pub fn push_python_attrs(
        &mut self,
        symbol_id: &str,
        decorators: &[String],
        is_generator: bool,
        is_coroutine: bool,
        docstring_style: Option<&str>,
    ) {
        self.python_attrs.push(vec![
            text(symbol_id),
            list_text(decorators),
            Value::Boolean(is_generator),
            Value::Boolean(is_coroutine),
            opt_text(docstring_style),
        ]);
    }

    pub fn push_typescript_attrs(
        &mut self,
        symbol_id: &str,
        is_readonly: bool,
        is_optional: bool,
        type_parameters: &[String],
    ) {
        self.typescript_attrs.push(vec![
            text(symbol_id),
            Value::Boolean(is_readonly),
            Value::Boolean(is_optional),
            list_text(type_parameters),
        ]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_cpp_attrs(
        &mut self,
        symbol_id: &str,
        is_virtual: bool,
        is_const: bool,
        is_noexcept: bool,
        is_template: bool,
        is_constexpr: bool,
        is_override: bool,
        is_final: bool,
    ) {
        self.cpp_attrs.push(vec![
            text(symbol_id),
            Value::Boolean(is_virtual),
            Value::Boolean(is_const),
            Value::Boolean(is_noexcept),
            Value::Boolean(is_template),
            Value::Boolean(is_constexpr),
            Value::Boolean(is_override),
            Value::Boolean(is_final),
        ]);
    }

    pub fn push_csharp_attrs(
        &mut self,
        symbol_id: &str,
        attributes: &[String],
        is_partial: bool,
        is_sealed: bool,
    ) {
        self.csharp_attrs.push(vec![
            text(symbol_id),
            list_text(attributes),
            Value::Boolean(is_partial),
            Value::Boolean(is_sealed),
        ]);
    }

    pub fn push_go_attrs(
        &mut self,
        symbol_id: &str,
        is_exported: bool,
        has_receiver: bool,
        build_tags: &[String],
    ) {
        self.go_attrs.push(vec![
            text(symbol_id),
            Value::Boolean(is_exported),
            Value::Boolean(has_receiver),
            list_text(build_tags),
        ]);
    }

    pub fn push_php_attrs(
        &mut self,
        symbol_id: &str,
        is_final: bool,
        uses_traits: &[String],
        attributes: &[String],
    ) {
        self.php_attrs.push(vec![
            text(symbol_id),
            Value::Boolean(is_final),
            list_text(uses_traits),
            list_text(attributes),
        ]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_c_attrs(
        &mut self,
        symbol_id: &str,
        is_file_static: bool,
        is_extern: bool,
        is_inline: bool,
        is_const: bool,
        is_volatile: bool,
        is_restrict: bool,
        gcc_attributes: &[String],
    ) {
        self.c_attrs.push(vec![
            text(symbol_id),
            Value::Boolean(is_file_static),
            Value::Boolean(is_extern),
            Value::Boolean(is_inline),
            Value::Boolean(is_const),
            Value::Boolean(is_volatile),
            Value::Boolean(is_restrict),
            list_text(gcc_attributes),
        ]);
    }

    pub fn push_java_attrs(
        &mut self,
        symbol_id: &str,
        annotations: &[String],
        is_final: bool,
        is_synchronized: bool,
        throws_clause: &[String],
    ) {
        self.java_attrs.push(vec![
            text(symbol_id),
            list_text(annotations),
            Value::Boolean(is_final),
            Value::Boolean(is_synchronized),
            list_text(throws_clause),
        ]);
    }

    /// Flush every buffered relation to `store`. Empty buffers are
    /// skipped.
    ///
    /// Each call passes the PK column count (always the first N
    /// columns of the row, matching the schema declaration). The
    /// flush helpers dedupe rows by their PK keys, keeping the last
    /// write — this matches Cozo's `:put` upsert semantics, which the
    /// extractors relied on (sometimes inadvertently) and DuckDB's
    /// strict Appender otherwise rejects with PK violations.
    pub fn flush(&mut self, store: &DbStore) -> Result<()> {
        store.with_conn(|conn| -> Result<()> {
            flush_table(conn, "file", 1, &mut self.file)?;
            flush_table(conn, "symbol", 1, &mut self.symbol)?;
            flush_table(conn, "span", 2, &mut self.span)?;
            flush_table(conn, "calls", 2, &mut self.calls)?;
            flush_table(conn, "call_site", 1, &mut self.call_site)?;
            flush_table(conn, "call_edge", 2, &mut self.call_edge)?;
            flush_table(conn, "extends", 2, &mut self.extends)?;
            flush_table(conn, "implements", 2, &mut self.implements)?;
            flush_table(conn, "raw_inheritance", 0, &mut self.raw_inheritance)?;
            flush_table(conn, "imports", 2, &mut self.imports)?;
            flush_table(conn, "raw_import", 2, &mut self.raw_import)?;
            flush_table(conn, "parameter", 1, &mut self.parameter)?;
            flush_table(conn, "returns_type", 1, &mut self.returns_type)?;
            flush_table(conn, "throws", 2, &mut self.throws)?;
            flush_table(conn, "field_type", 1, &mut self.field_type)?;
            flush_table(conn, "type", 1, &mut self.ty)?;
            flush_table(conn, "comment", 1, &mut self.comment)?;
            flush_table(
                conn,
                "file_classification",
                1,
                &mut self.file_classification,
            )?;
            flush_table(conn, "nolint", 2, &mut self.nolint)?;
            flush_table(conn, "build_meta", 1, &mut self.build_meta)?;
            flush_table(conn, "build_meta_files", 1, &mut self.build_meta_files)?;
            flush_table(conn, "occurrence", 1, &mut self.occurrence)?;
            flush_table(conn, "scope", 1, &mut self.scope)?;
            flush_table(conn, "binding", 3, &mut self.binding)?;
            flush_table(conn, "local_type", 3, &mut self.local_type)?;
            // Attrs tables have VARCHAR[] columns. The duckdb crate's
            // Appender path goes through `ValueRef::from(Value)`, which
            // is `unimplemented!()` for `Value::List` in duckdb 1.2.
            // Route them through a batched literal-inline INSERT
            // instead — one round trip per table.
            flush_table_with_arrays(conn, "rust_attrs", 1, &mut self.rust_attrs)?;
            flush_table_with_arrays(conn, "python_attrs", 1, &mut self.python_attrs)?;
            flush_table_with_arrays(conn, "typescript_attrs", 1, &mut self.typescript_attrs)?;
            flush_table(conn, "cpp_attrs", 1, &mut self.cpp_attrs)?;
            flush_table_with_arrays(conn, "csharp_attrs", 1, &mut self.csharp_attrs)?;
            flush_table_with_arrays(conn, "go_attrs", 1, &mut self.go_attrs)?;
            flush_table_with_arrays(conn, "php_attrs", 1, &mut self.php_attrs)?;
            flush_table_with_arrays(conn, "c_attrs", 1, &mut self.c_attrs)?;
            flush_table_with_arrays(conn, "java_attrs", 1, &mut self.java_attrs)?;
            Ok(())
        })
    }
}

/// Dedupe `rows` by the first `pk_cols` columns, keeping the LAST
/// occurrence of each key. Mirrors Cozo's `:put` upsert semantics —
/// extractors that emit overlapping rows (e.g. nested-symbol
/// duplicates) rely on this to land cleanly under DuckDB's strict
/// Appender path. O(n) using a HashMap of seen keys.
fn dedupe_by_pk_keep_last(rows: &mut Vec<Row>, pk_cols: usize) {
    if rows.len() < 2 || pk_cols == 0 {
        return;
    }
    use std::collections::HashMap;
    // `duckdb::types::Value` doesn't implement Hash/Eq (NaN floats),
    // so render the PK cells as strings for the key. PK columns are
    // all VARCHAR / BIGINT / BOOLEAN in our schema so the rendering
    // is unambiguous.
    let mut last_index: HashMap<String, usize> = HashMap::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let key = pk_key(row, pk_cols);
        last_index.insert(key, i);
    }
    if last_index.len() == rows.len() {
        return;
    }
    let mut keep: Vec<bool> = vec![false; rows.len()];
    for &i in last_index.values() {
        keep[i] = true;
    }
    let mut idx = 0;
    rows.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

fn pk_key(row: &Row, pk_cols: usize) -> String {
    let mut s = String::with_capacity(64);
    for v in row.iter().take(pk_cols) {
        match v {
            Value::Text(t) => {
                s.push('\u{1F}'); // unit separator, won't collide with content
                s.push_str(t);
            }
            Value::BigInt(n) => {
                s.push('\u{1F}');
                s.push_str(&n.to_string());
            }
            Value::Int(n) => {
                s.push('\u{1F}');
                s.push_str(&n.to_string());
            }
            Value::Boolean(b) => {
                s.push('\u{1F}');
                s.push(if *b { 't' } else { 'f' });
            }
            Value::Null => {
                s.push('\u{1F}');
                s.push('\u{0}');
            }
            other => {
                s.push('\u{1F}');
                s.push_str(&format!("{other:?}"));
            }
        }
    }
    s
}

fn flush_table(conn: &Connection, table: &str, pk_cols: usize, rows: &mut Vec<Row>) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    dedupe_by_pk_keep_last(rows, pk_cols);
    let mut app = conn
        .appender(table)
        .map_err(|e| anyhow!("opening appender for {table}: {e}"))?;
    for row in rows.drain(..) {
        app.append_row(duckdb::appender_params_from_iter(row.iter()))
            .map_err(|e| anyhow!("append_row into {table}: {e}"))?;
    }
    app.flush()
        .map_err(|e| anyhow!("flushing appender for {table}: {e}"))?;
    Ok(())
}

/// Flush a table that has at least one `VARCHAR[]` column via a
/// batched `INSERT INTO t VALUES (...), (...), ...` with values
/// rendered as SQL literals. Slower per-row than Appender, but the
/// attrs tables are sparse so the total cost stays small.
fn flush_table_with_arrays(
    conn: &Connection,
    table: &str,
    pk_cols: usize,
    rows: &mut Vec<Row>,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    dedupe_by_pk_keep_last(rows, pk_cols);
    for chunk in rows.chunks(FLUSH_BATCH) {
        let mut sql = format!("INSERT INTO {table} VALUES ");
        for (i, row) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push('(');
            for (j, v) in row.iter().enumerate() {
                if j > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&value_to_sql_literal(v));
            }
            sql.push(')');
        }
        conn.execute(&sql, [])
            .map_err(|e| anyhow!("batch insert into {table}: {e}"))?;
    }
    rows.clear();
    Ok(())
}

fn value_to_sql_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".to_string(),
        Value::Boolean(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        Value::TinyInt(n) => n.to_string(),
        Value::SmallInt(n) => n.to_string(),
        Value::Int(n) => n.to_string(),
        Value::BigInt(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Double(n) => n.to_string(),
        Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(value_to_sql_literal).collect();
            format!("[{}]", parts.join(", "))
        }
        other => panic!("flush_table_with_arrays: unsupported value variant {other:?}"),
    }
}

#[inline]
fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

#[inline]
fn big(n: i64) -> Value {
    Value::BigInt(n)
}

#[inline]
fn opt_text(s: Option<&str>) -> Value {
    match s {
        Some(s) => Value::Text(s.to_string()),
        None => Value::Null,
    }
}

#[inline]
fn list_text(items: &[String]) -> Value {
    Value::List(items.iter().map(|s| Value::Text(s.clone())).collect())
}

// Quiet the unused-import warning when no tests are compiled.
#[allow(dead_code)]
fn _unused(_: BTreeMap<String, Value>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_round_trips_symbols_and_calls() {
        let store = DbStore::open_in_memory().expect("open");
        let mut writer = DbWriter::new();

        writer.push_file("src/a.ts", "typescript", "");
        writer.push_symbol(
            "src/a.ts|1|0|login|function",
            "function",
            "login",
            "login",
            "typescript",
            "public",
            "src/a.ts",
            None,
            false,
            false,
            false,
            false,
            true,
        );
        writer.push_symbol(
            "src/a.ts|11|0|checkPassword|function",
            "function",
            "checkPassword",
            "checkPassword",
            "typescript",
            "private",
            "src/a.ts",
            None,
            false,
            false,
            false,
            false,
            false,
        );
        writer.push_calls(
            "src/a.ts|1|0|login|function",
            "src/a.ts|11|0|checkPassword|function",
            "src/a.ts",
            42,
            55,
            true,
        );

        writer.flush(&store).expect("flush");

        let rows = store
            .run_query(
                "SELECT s_caller.name, s_callee.name \
                 FROM calls c \
                 JOIN symbol s_caller ON s_caller.id = c.caller_id \
                 JOIN symbol s_callee ON s_callee.id = c.callee_id",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0], Value::Text("login".into()));
        assert_eq!(rows.rows[0][1], Value::Text("checkPassword".into()));
    }

    #[test]
    fn flush_writes_call_edge_rows() {
        let store = DbStore::open_in_memory().expect("open");
        let mut w = DbWriter::new();
        w.push_call_edge("caller-id-1", "callee-id-1", "src/a.rs");
        w.push_call_edge("caller-id-2", "callee-id-2", "src/b.rs");
        w.flush(&store).expect("flush");

        let rows = store
            .run_query(
                "SELECT caller_id, callee_id, file_path FROM call_edge ORDER BY caller_id",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 2);
    }

    #[test]
    fn writer_pushes_attrs_with_list_columns() {
        let store = DbStore::open_in_memory().expect("open");
        let mut w = DbWriter::new();
        w.push_file("src/lib.rs", "rust", "");
        w.push_symbol(
            "src/lib.rs|1|0|foo|function",
            "function",
            "foo",
            "foo",
            "rust",
            "public",
            "src/lib.rs",
            None,
            false,
            false,
            false,
            false,
            true,
        );
        w.push_rust_attrs(
            "src/lib.rs|1|0|foo|function",
            true,
            false,
            &["Debug".to_string(), "Clone".to_string()],
        );
        w.flush(&store).expect("flush");
        let rows = store
            .run_query(
                "SELECT len(derives) FROM rust_attrs WHERE symbol_id = 'src/lib.rs|1|0|foo|function'",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0], Value::BigInt(2));
    }
}
