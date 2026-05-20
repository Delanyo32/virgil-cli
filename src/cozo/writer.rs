//! Batched row writer fed by the graph absorber.
//!
//! Rows are accumulated per-relation and flushed in chunks (~10k rows per
//! transaction) so transaction overhead stays low without losing per-row
//! atomicity expectations.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;

use super::store::CozoStore;

const FLUSH_BATCH: usize = 10_000;

/// Accumulates per-relation rows and flushes them to a [`CozoStore`].
pub struct CozoWriter {
    file: Vec<Vec<DataValue>>,
    symbol: Vec<Vec<DataValue>>,
    callsite: Vec<Vec<DataValue>>,
    edge_defined_in: Vec<Vec<DataValue>>,
    edge_calls: Vec<Vec<DataValue>>,
    edge_imports: Vec<Vec<DataValue>>,
    edge_exports: Vec<Vec<DataValue>>,
    edge_contains: Vec<Vec<DataValue>>,
    raw_import: Vec<Vec<DataValue>>,
    // ---- Derived facts (issue 04) ----
    file_classification: Vec<Vec<DataValue>>,
    nolint: Vec<Vec<DataValue>>,
    // ---- Metadata ----
    build_meta_files: Vec<Vec<DataValue>>,
}

impl CozoWriter {
    pub fn new() -> Self {
        Self {
            file: Vec::new(),
            symbol: Vec::new(),
            callsite: Vec::new(),
            edge_defined_in: Vec::new(),
            edge_calls: Vec::new(),
            edge_imports: Vec::new(),
            edge_exports: Vec::new(),
            edge_contains: Vec::new(),
            raw_import: Vec::new(),
            file_classification: Vec::new(),
            nolint: Vec::new(),
            build_meta_files: Vec::new(),
        }
    }

    /// Append every row from `other` into `self`, leaving `other` empty.
    /// Used by the parallel populate path so per-thread writers can be
    /// merged before a single flush call.
    pub fn merge(&mut self, other: &mut CozoWriter) {
        self.file.append(&mut other.file);
        self.symbol.append(&mut other.symbol);
        self.callsite.append(&mut other.callsite);
        self.edge_defined_in.append(&mut other.edge_defined_in);
        self.edge_calls.append(&mut other.edge_calls);
        self.edge_imports.append(&mut other.edge_imports);
        self.edge_exports.append(&mut other.edge_exports);
        self.edge_contains.append(&mut other.edge_contains);
        self.raw_import.append(&mut other.raw_import);
        self.file_classification.append(&mut other.file_classification);
        self.nolint.append(&mut other.nolint);
        self.build_meta_files.append(&mut other.build_meta_files);
    }

    pub fn push_file(&mut self, path: &str, language: &str) {
        self.file
            .push(vec![DataValue::from(path), DataValue::from(language)]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_symbol(
        &mut self,
        id: i64,
        name: &str,
        kind: &str,
        file_path: &str,
        start_line: i64,
        end_line: i64,
        exported: bool,
    ) {
        self.symbol.push(vec![
            DataValue::from(id),
            DataValue::from(name),
            DataValue::from(kind),
            DataValue::from(file_path),
            DataValue::from(start_line),
            DataValue::from(end_line),
            DataValue::from(exported),
        ]);
    }

    pub fn push_callsite(
        &mut self,
        id: i64,
        name: &str,
        file_path: &str,
        line: i64,
        caller_symbol_id: Option<i64>,
        enclosing_test_name: Option<&str>,
    ) {
        self.callsite.push(vec![
            DataValue::from(id),
            DataValue::from(name),
            DataValue::from(file_path),
            DataValue::from(line),
            caller_symbol_id
                .map(DataValue::from)
                .unwrap_or(DataValue::Null),
            enclosing_test_name
                .map(DataValue::from)
                .unwrap_or(DataValue::Null),
        ]);
    }

    pub fn push_edge_defined_in(&mut self, symbol_id: i64, file_path: &str) {
        self.edge_defined_in
            .push(vec![DataValue::from(symbol_id), DataValue::from(file_path)]);
    }

    pub fn push_edge_calls(&mut self, caller_id: i64, callee_id: i64) {
        self.edge_calls
            .push(vec![DataValue::from(caller_id), DataValue::from(callee_id)]);
    }

    pub fn push_edge_imports(&mut self, from_path: &str, to_path: &str) {
        self.edge_imports
            .push(vec![DataValue::from(from_path), DataValue::from(to_path)]);
    }

    pub fn push_edge_exports(&mut self, file_path: &str, symbol_id: i64) {
        self.edge_exports
            .push(vec![DataValue::from(file_path), DataValue::from(symbol_id)]);
    }

    pub fn push_edge_contains(&mut self, parent_id: i64, child_id: i64) {
        self.edge_contains
            .push(vec![DataValue::from(parent_id), DataValue::from(child_id)]);
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
            "?[path, language] <- $rows :put file {path => language}",
            std::mem::take(&mut self.file),
        )?;
        flush(
            store,
            "?[id, name, kind, file_path, start_line, end_line, exported] <- $rows \
             :put symbol {id => name, kind, file_path, start_line, end_line, exported}",
            std::mem::take(&mut self.symbol),
        )?;
        flush(
            store,
            "?[id, name, file_path, line, caller_symbol_id, enclosing_test_name] <- $rows \
             :put callsite {id => name, file_path, line, caller_symbol_id, enclosing_test_name}",
            std::mem::take(&mut self.callsite),
        )?;
        flush(
            store,
            "?[symbol_id, file_path] <- $rows :put edge_defined_in {symbol_id, file_path}",
            std::mem::take(&mut self.edge_defined_in),
        )?;
        flush(
            store,
            "?[caller_id, callee_id] <- $rows :put edge_calls {caller_id, callee_id}",
            std::mem::take(&mut self.edge_calls),
        )?;
        flush(
            store,
            "?[from_path, to_path] <- $rows :put edge_imports {from_path, to_path}",
            std::mem::take(&mut self.edge_imports),
        )?;
        flush(
            store,
            "?[file_path, symbol_id] <- $rows :put edge_exports {file_path, symbol_id}",
            std::mem::take(&mut self.edge_exports),
        )?;
        flush(
            store,
            "?[parent_id, child_id] <- $rows :put edge_contains {parent_id, child_id}",
            std::mem::take(&mut self.edge_contains),
        )?;
        flush(
            store,
            "?[file_path, position, raw_path, language, kind] <- $rows \
             :put raw_import {file_path, position => raw_path, language, kind}",
            std::mem::take(&mut self.raw_import),
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
            "?[file_path, hash, size, mtime] <- $rows \
             :put build_meta_files {file_path => hash, size, mtime}",
            std::mem::take(&mut self.build_meta_files),
        )?;
        Ok(())
    }
}

impl Default for CozoWriter {
    fn default() -> Self {
        Self::new()
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

        writer.push_file("src/a.ts", "typescript");
        writer.push_symbol(1, "login", "function", "src/a.ts", 1, 10, true);
        writer.push_symbol(2, "checkPassword", "function", "src/a.ts", 11, 20, false);
        writer.push_edge_defined_in(1, "src/a.ts");
        writer.push_edge_defined_in(2, "src/a.ts");
        writer.push_edge_calls(1, 2);
        writer.push_edge_exports("src/a.ts", 1);

        writer.flush(&store).expect("flush");

        let calls = store
            .run_query(
                "?[caller, callee] := \
                 *edge_calls{caller_id, callee_id}, \
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
    fn writer_handles_nullable_callsite_fields() {
        let store = CozoStore::open_in_memory().expect("open");
        let mut writer = CozoWriter::new();

        writer.push_callsite(1, "fetch", "src/api.ts", 42, None, None);
        writer.push_callsite(2, "fetch", "src/api.ts", 50, Some(99), Some("it works"));
        writer.flush(&store).expect("flush");

        let rows = store
            .run_query(
                "?[name, line] := *callsite{name, line}",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 2);
    }
}
