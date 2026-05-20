//! Batched row writer fed by the graph absorber.
//!
//! Rows are accumulated per-relation and flushed in chunks (~10k rows per
//! transaction) so transaction overhead stays low without losing per-row
//! atomicity expectations.
//!
//! Phase 1: all IDs are `String` per [ADR-0002]. Per-language `*_attrs`
//! tables and the new graph relations (`extends`, `implements`,
//! `references`, `field_type`, `type`, `comment`, `span`) have push methods
//! and a corresponding `flush` line each; most stay empty until later
//! phases fill them.
//!
//! [ADR-0002]: docs/adr/0002-symbol-id-scheme.md

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;

use super::store::CozoStore;

const FLUSH_BATCH: usize = 10_000;

/// Accumulates per-relation rows and flushes them to a [`CozoStore`].
#[derive(Default)]
pub struct CozoWriter {
    file: Vec<Vec<DataValue>>,
    symbol: Vec<Vec<DataValue>>,
    span: Vec<Vec<DataValue>>,
    calls: Vec<Vec<DataValue>>,
    references: Vec<Vec<DataValue>>,
    extends: Vec<Vec<DataValue>>,
    implements: Vec<Vec<DataValue>>,
    imports: Vec<Vec<DataValue>>,
    raw_import: Vec<Vec<DataValue>>,
    parameter: Vec<Vec<DataValue>>,
    returns_type: Vec<Vec<DataValue>>,
    throws: Vec<Vec<DataValue>>,
    field_type: Vec<Vec<DataValue>>,
    ty: Vec<Vec<DataValue>>,
    comment: Vec<Vec<DataValue>>,
    file_classification: Vec<Vec<DataValue>>,
    nolint: Vec<Vec<DataValue>>,
    build_meta: Vec<Vec<DataValue>>,
    build_meta_files: Vec<Vec<DataValue>>,
}

impl CozoWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append every row from `other` into `self`, leaving `other` empty.
    /// Used by the parallel populate path so per-thread writers can be
    /// merged before a single flush call.
    pub fn merge(&mut self, other: &mut CozoWriter) {
        self.file.append(&mut other.file);
        self.symbol.append(&mut other.symbol);
        self.span.append(&mut other.span);
        self.calls.append(&mut other.calls);
        self.references.append(&mut other.references);
        self.extends.append(&mut other.extends);
        self.implements.append(&mut other.implements);
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
    }

    pub fn push_file(&mut self, path: &str, language: &str, repo_id: &str) {
        self.file.push(vec![
            DataValue::from(path),
            DataValue::from(language),
            DataValue::from(repo_id),
        ]);
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
            DataValue::from(id),
            DataValue::from(kind),
            DataValue::from(name),
            DataValue::from(qualified_name),
            DataValue::from(language),
            DataValue::from(visibility),
            DataValue::from(file_path),
            parent_id.map(DataValue::from).unwrap_or(DataValue::Null),
            DataValue::from(is_async),
            DataValue::from(is_static),
            DataValue::from(is_abstract),
            DataValue::from(is_mutable),
            DataValue::from(exported),
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
            DataValue::from(entity_id),
            DataValue::from(file_path),
            DataValue::from(start_byte),
            DataValue::from(end_byte),
            DataValue::from(start_line),
            DataValue::from(end_line),
            DataValue::from(start_col),
            DataValue::from(end_col),
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
            DataValue::from(caller_id),
            DataValue::from(callee_id),
            DataValue::from(call_site_file),
            DataValue::from(call_site_start_byte),
            DataValue::from(call_site_end_byte),
            DataValue::from(is_direct),
        ]);
    }

    pub fn push_references(
        &mut self,
        referrer_id: &str,
        site_file: &str,
        site_start_byte: i64,
        match_index: i64,
        referent_id: Option<&str>,
        ref_kind: &str,
    ) {
        self.references.push(vec![
            DataValue::from(referrer_id),
            DataValue::from(site_file),
            DataValue::from(site_start_byte),
            DataValue::from(match_index),
            referent_id.map(DataValue::from).unwrap_or(DataValue::Null),
            DataValue::from(ref_kind),
        ]);
    }

    pub fn push_extends(&mut self, child_id: &str, parent_id: &str) {
        self.extends
            .push(vec![DataValue::from(child_id), DataValue::from(parent_id)]);
    }

    pub fn push_implements(&mut self, impl_id: &str, interface_id: &str) {
        self.implements
            .push(vec![DataValue::from(impl_id), DataValue::from(interface_id)]);
    }

    pub fn push_imports(&mut self, importer_file_id: &str, imported_id: &str) {
        self.imports.push(vec![
            DataValue::from(importer_file_id),
            DataValue::from(imported_id),
        ]);
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
            DataValue::from(file_path),
            DataValue::from(position),
            DataValue::from(raw_path),
            DataValue::from(language),
            DataValue::from(kind),
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
            DataValue::from(id),
            DataValue::from(name),
            DataValue::from(function_id),
            DataValue::from(position),
            type_id.map(DataValue::from).unwrap_or(DataValue::Null),
            DataValue::from(is_optional),
            DataValue::from(has_default),
            DataValue::from(is_taint_source),
        ]);
    }

    pub fn push_returns_type(&mut self, function_id: &str, type_id: &str) {
        self.returns_type
            .push(vec![DataValue::from(function_id), DataValue::from(type_id)]);
    }

    pub fn push_throws(&mut self, function_id: &str, exception_type_id: &str) {
        self.throws.push(vec![
            DataValue::from(function_id),
            DataValue::from(exception_type_id),
        ]);
    }

    pub fn push_field_type(&mut self, symbol_id: &str, type_id: &str) {
        self.field_type
            .push(vec![DataValue::from(symbol_id), DataValue::from(type_id)]);
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
            DataValue::from(id),
            DataValue::from(kind),
            DataValue::from(language),
            DataValue::from(display_name),
            canonical_name
                .map(DataValue::from)
                .unwrap_or(DataValue::Null),
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
        text: &str,
        todo_kind: Option<&str>,
        start_byte: i64,
        end_byte: i64,
    ) {
        self.comment.push(vec![
            DataValue::from(id),
            documents_id.map(DataValue::from).unwrap_or(DataValue::Null),
            DataValue::from(file_path),
            DataValue::from(kind),
            DataValue::from(is_doc),
            DataValue::from(text),
            todo_kind.map(DataValue::from).unwrap_or(DataValue::Null),
            DataValue::from(start_byte),
            DataValue::from(end_byte),
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
            DataValue::from(path),
            DataValue::from(is_test),
            DataValue::from(is_barrel),
            DataValue::from(is_generated),
        ]);
    }

    pub fn push_nolint(&mut self, file_path: &str, line: i64, suppressed_pattern: &str) {
        self.nolint.push(vec![
            DataValue::from(file_path),
            DataValue::from(line),
            DataValue::from(suppressed_pattern),
        ]);
    }

    pub fn push_build_meta(&mut self, key: &str, value: &str) {
        self.build_meta
            .push(vec![DataValue::from(key), DataValue::from(value)]);
    }

    pub fn push_build_meta_file(&mut self, file_path: &str, hash: &str, size: i64, mtime: i64) {
        self.build_meta_files.push(vec![
            DataValue::from(file_path),
            DataValue::from(hash),
            DataValue::from(size),
            DataValue::from(mtime),
        ]);
    }

    /// Flush every buffered relation to `store`. Empty buffers are skipped.
    pub fn flush(&mut self, store: &CozoStore) -> Result<()> {
        flush(
            store,
            "?[path, language, repo_id] <- $rows \
             :put file {path => language, repo_id}",
            std::mem::take(&mut self.file),
        )?;
        flush(
            store,
            "?[id, kind, name, qualified_name, language, visibility, file_path, \
              parent_id, is_async, is_static, is_abstract, is_mutable, exported] <- $rows \
             :put symbol {id => kind, name, qualified_name, language, visibility, \
                          file_path, parent_id, is_async, is_static, is_abstract, \
                          is_mutable, exported}",
            std::mem::take(&mut self.symbol),
        )?;
        flush(
            store,
            "?[entity_id, file_path, start_byte, end_byte, start_line, end_line, \
              start_col, end_col] <- $rows \
             :put span {entity_id, file_path => start_byte, end_byte, start_line, \
                        end_line, start_col, end_col}",
            std::mem::take(&mut self.span),
        )?;
        flush(
            store,
            "?[caller_id, callee_id, call_site_file, call_site_start_byte, \
              call_site_end_byte, is_direct] <- $rows \
             :put calls {caller_id, callee_id => call_site_file, \
                         call_site_start_byte, call_site_end_byte, is_direct}",
            std::mem::take(&mut self.calls),
        )?;
        flush(
            store,
            "?[referrer_id, site_file, site_start_byte, match_index, \
              referent_id, ref_kind] <- $rows \
             :put references {referrer_id, site_file, site_start_byte, match_index \
                              => referent_id, ref_kind}",
            std::mem::take(&mut self.references),
        )?;
        flush(
            store,
            "?[child_id, parent_id] <- $rows :put extends {child_id, parent_id}",
            std::mem::take(&mut self.extends),
        )?;
        flush(
            store,
            "?[impl_id, interface_id] <- $rows :put implements {impl_id, interface_id}",
            std::mem::take(&mut self.implements),
        )?;
        flush(
            store,
            "?[importer_file_id, imported_id] <- $rows \
             :put imports {importer_file_id, imported_id}",
            std::mem::take(&mut self.imports),
        )?;
        flush(
            store,
            "?[file_path, position, raw_path, language, kind] <- $rows \
             :put raw_import {file_path, position => raw_path, language, kind}",
            std::mem::take(&mut self.raw_import),
        )?;
        flush(
            store,
            "?[id, name, function_id, position, type_id, is_optional, has_default, \
              is_taint_source] <- $rows \
             :put parameter {id => name, function_id, position, type_id, \
                             is_optional, has_default, is_taint_source}",
            std::mem::take(&mut self.parameter),
        )?;
        flush(
            store,
            "?[function_id, type_id] <- $rows :put returns_type {function_id => type_id}",
            std::mem::take(&mut self.returns_type),
        )?;
        flush(
            store,
            "?[function_id, exception_type_id] <- $rows \
             :put throws {function_id, exception_type_id}",
            std::mem::take(&mut self.throws),
        )?;
        flush(
            store,
            "?[symbol_id, type_id] <- $rows :put field_type {symbol_id => type_id}",
            std::mem::take(&mut self.field_type),
        )?;
        flush(
            store,
            "?[id, kind, language, display_name, canonical_name] <- $rows \
             :put type {id => kind, language, display_name, canonical_name}",
            std::mem::take(&mut self.ty),
        )?;
        flush(
            store,
            "?[id, documents_id, file_path, kind, is_doc, text, todo_kind, \
              start_byte, end_byte] <- $rows \
             :put comment {id => documents_id, file_path, kind, is_doc, text, \
                           todo_kind, start_byte, end_byte}",
            std::mem::take(&mut self.comment),
        )?;
        flush(
            store,
            "?[path, is_test, is_barrel, is_generated] <- $rows \
             :put file_classification {path => is_test, is_barrel, is_generated}",
            std::mem::take(&mut self.file_classification),
        )?;
        flush(
            store,
            "?[file_path, line, suppressed_pattern] <- $rows \
             :put nolint {file_path, line => suppressed_pattern}",
            std::mem::take(&mut self.nolint),
        )?;
        flush(
            store,
            "?[key, value] <- $rows :put build_meta {key => value}",
            std::mem::take(&mut self.build_meta),
        )?;
        flush(
            store,
            "?[file_path, hash, size, mtime] <- $rows \
             :put build_meta_files {file_path => hash, size, mtime}",
            std::mem::take(&mut self.build_meta_files),
        )?;
        Ok(())
    }
}

fn flush(store: &CozoStore, script: &str, rows: Vec<Vec<DataValue>>) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    for chunk in rows.chunks(FLUSH_BATCH) {
        let batch: Vec<DataValue> = chunk
            .iter()
            .map(|row| DataValue::List(row.clone()))
            .collect();
        let mut params = BTreeMap::new();
        params.insert("rows".to_string(), DataValue::List(batch));
        store.run_script(script, params)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_round_trips_symbols_and_calls() {
        let store = CozoStore::open_in_memory().expect("open");
        let mut writer = CozoWriter::new();

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

        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *calls{caller_id, callee_id}, \
                 *symbol{id: caller_id, name: caller}, \
                 *symbol{id: callee_id, name: callee}",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(calls.rows.len(), 1);
        assert_eq!(calls.rows[0][0], DataValue::from("login"));
        assert_eq!(calls.rows[0][1], DataValue::from("checkPassword"));
    }

    #[test]
    fn writer_handles_nullable_references() {
        let store = CozoStore::open_in_memory().expect("open");
        let mut writer = CozoWriter::new();

        // unresolved reference → referent_id = null
        writer.push_references("caller-1", "src/api.ts", 100, 0, None, "read");
        // overload candidate 0
        writer.push_references("caller-2", "src/api.ts", 200, 0, Some("target-a"), "read");
        // overload candidate 1 (same site, different match_index)
        writer.push_references("caller-2", "src/api.ts", 200, 1, Some("target-b"), "read");

        writer.flush(&store).expect("flush");

        let rows = store
            .run_query(
                "?[r, m, ref] := *references{referrer_id: r, match_index: m, referent_id: ref}",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 3);
    }
}
