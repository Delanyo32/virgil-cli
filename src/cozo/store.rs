//! Thin wrapper around `cozo::DbInstance`. Owns lifecycle and exposes
//! `run_script` / `run_query` helpers that apply parameter binding
//! consistently.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use cozo::{DataValue, DbInstance, NamedRows};

use super::SCHEMA_VERSION;
use super::schema;

/// A Cozo database handle. At this stage backed by an in-memory store; the
/// on-disk variants are wired up in issue 07.
pub struct CozoStore {
    db: DbInstance,
}

impl CozoStore {
    /// Open a fresh in-memory store with the cross-function schema applied
    /// and the schema version recorded in `build_meta`.
    pub fn open_in_memory() -> Result<Self> {
        let db = DbInstance::new("mem", "", Default::default())
            .map_err(|e| anyhow!("failed to open cozo mem store: {e}"))?;
        let store = Self { db };
        store.apply_schema()?;
        store.record_schema_version()?;
        Ok(store)
    }

    fn apply_schema(&self) -> Result<()> {
        for stmt in schema::create_statements() {
            self.run_script(stmt, BTreeMap::new())
                .with_context(|| format!("applying schema statement: {stmt}"))?;
        }
        for stmt in schema::index_statements() {
            self.run_script(stmt, BTreeMap::new())
                .with_context(|| format!("applying index statement: {stmt}"))?;
        }
        Ok(())
    }

    fn record_schema_version(&self) -> Result<()> {
        let mut params = BTreeMap::new();
        params.insert(
            "v".to_string(),
            DataValue::from(SCHEMA_VERSION.to_string()),
        );
        self.run_script(
            "?[key, value] <- [['schema_version', $v]] :put build_meta {key => value}",
            params,
        )?;
        Ok(())
    }

    /// Run a mutating Cozoscript snippet with the provided parameter map.
    pub fn run_script(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
    ) -> Result<NamedRows> {
        self.db
            .run_script(script, params, cozo::ScriptMutability::Mutable)
            .map_err(|e| anyhow!("cozo script failed: {e}"))
    }

    /// Read-only variant. Use for queries that must not mutate.
    pub fn run_query(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
    ) -> Result<NamedRows> {
        self.db
            .run_script(script, params, cozo::ScriptMutability::Immutable)
            .map_err(|e| anyhow!("cozo query failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_opens_and_records_schema_version() {
        let store = CozoStore::open_in_memory().expect("open");
        let rows = store
            .run_query(
                "?[v] := *build_meta{key: 'schema_version', value: v}",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(
            rows.rows[0][0],
            DataValue::from(SCHEMA_VERSION.to_string())
        );
    }
}
