//! Thin wrapper around `cozo::DbInstance`. Owns lifecycle and exposes
//! `run_script` / `run_query` helpers that apply parameter binding
//! consistently.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use cozo::{DataValue, DbInstance, NamedRows};

use super::SCHEMA_VERSION;
use super::schema;

/// A Cozo database handle. Two open paths:
/// - [`CozoStore::open_in_memory`] — ephemeral; used by tests.
/// - [`CozoStore::open_persistent`] — SQLite-backed, cached on disk under
///   [`cache_dir_for`]. Used by the CLI + serve.
pub struct CozoStore {
    db: DbInstance,
    /// `true` when the schema was applied fresh during this open (i.e. the
    /// caller must populate the store from a workspace before querying).
    /// `false` when an existing store was reopened.
    fresh: bool,
}

impl CozoStore {
    /// Open a fresh in-memory store with the cross-function schema applied
    /// and the schema version recorded in `build_meta`.
    pub fn open_in_memory() -> Result<Self> {
        let db = DbInstance::new("mem", "", Default::default())
            .map_err(|e| anyhow!("failed to open cozo mem store: {e}"))?;
        let store = Self { db, fresh: true };
        store.apply_schema()?;
        store.record_schema_version()?;
        Ok(store)
    }

    /// Open a SQLite-backed store at `path`. If the file exists, reopen it
    /// in place (no schema re-apply). If the schema version doesn't match
    /// [`SCHEMA_VERSION`], the file is removed and recreated from scratch.
    /// Returns the store + a flag indicating whether the caller still needs
    /// to populate it (via [`fresh`](Self::fresh)).
    pub fn open_persistent(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating cache dir {}", parent.display()))?;
        }

        if path.exists() {
            match Self::try_reopen(path)? {
                Some(store) => return Ok(store),
                None => {
                    // Schema mismatch — wipe and recreate.
                    std::fs::remove_file(path)
                        .with_context(|| format!("removing stale store {}", path.display()))?;
                }
            }
        }

        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("non-utf8 cache path: {}", path.display()))?;
        let db = DbInstance::new("sqlite", path_str, Default::default())
            .map_err(|e| anyhow!("failed to open cozo sqlite store: {e}"))?;
        let store = Self { db, fresh: true };
        store.apply_schema()?;
        store.record_schema_version()?;
        Ok(store)
    }

    /// Attempt to reopen an existing store. Returns `Ok(Some(store))` on
    /// schema match, `Ok(None)` if the version doesn't match (caller will
    /// recreate), and `Err` on real I/O failure.
    fn try_reopen(path: &Path) -> Result<Option<Self>> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("non-utf8 cache path: {}", path.display()))?;
        let db = DbInstance::new("sqlite", path_str, Default::default())
            .map_err(|e| anyhow!("failed to reopen cozo sqlite store: {e}"))?;
        let store = Self { db, fresh: false };
        let version = store
            .run_query(
                "?[v] := *build_meta{key: 'schema_version', value: v}",
                BTreeMap::new(),
            )
            .ok();
        let matches = version
            .and_then(|rows| rows.rows.into_iter().next())
            .and_then(|r| r.into_iter().next())
            .map(|v| v == DataValue::from(SCHEMA_VERSION.to_string()))
            .unwrap_or(false);
        if matches {
            Ok(Some(store))
        } else {
            Ok(None)
        }
    }

    /// Whether the store was just created (or recreated). When `true` the
    /// caller must populate it from a workspace before queries make sense.
    pub fn fresh(&self) -> bool {
        self.fresh
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

/// Cache file path for a workspace identified by `id`. Returns
/// `~/.cache/virgil/<hash>.cozo` (or the platform cache dir equivalent).
///
/// `id` should uniquely and stably identify the workspace — for registered
/// projects, the project name; for S3, the full URI; for ad-hoc paths, the
/// canonical absolute path.
pub fn cache_dir_for(id: &str) -> Result<PathBuf> {
    let base = dirs::cache_dir()
        .context("could not determine OS cache directory")?
        .join("virgil");
    let hash = stable_hash(id);
    Ok(base.join(format!("{hash:016x}.cozo")))
}

fn stable_hash(s: &str) -> u64 {
    // FNV-1a 64. Stable across processes/platforms — DefaultHasher is not.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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

    #[test]
    fn persistent_store_round_trips_through_disk() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("test.cozo");

        let store = CozoStore::open_persistent(&path).expect("open fresh");
        assert!(store.fresh(), "expected fresh open");
        store
            .run_script(
                "?[id, name, kind, file_path, start_line, end_line, exported] <- \
                 [[1, 'login', 'function', 'a.ts', 1, 5, true]] \
                 :put symbol {id => name, kind, file_path, start_line, end_line, exported}",
                BTreeMap::new(),
            )
            .expect("insert");
        drop(store);

        // Reopen — should be warm, no fresh schema apply.
        let store = CozoStore::open_persistent(&path).expect("reopen");
        assert!(!store.fresh(), "expected warm reopen");
        let rows = store
            .run_query(
                "?[name] := *symbol{name}",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0], DataValue::from("login"));
    }

    #[test]
    fn schema_version_mismatch_triggers_clean_rebuild() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("test.cozo");

        // Open + write a bogus schema_version row that doesn't match.
        let store = CozoStore::open_persistent(&path).expect("open fresh");
        store
            .run_script(
                "?[key, value] <- [['schema_version', '999']] \
                 :put build_meta {key => value}",
                BTreeMap::new(),
            )
            .expect("overwrite version");
        drop(store);

        // Reopen — version mismatch should force rebuild (fresh = true).
        let store = CozoStore::open_persistent(&path).expect("reopen");
        assert!(store.fresh(), "expected fresh open after version mismatch");
    }

    #[test]
    fn cache_dir_is_stable_and_hashes_distinct_ids() {
        let a = cache_dir_for("project-a").unwrap();
        let a_again = cache_dir_for("project-a").unwrap();
        let b = cache_dir_for("project-b").unwrap();
        assert_eq!(a, a_again);
        assert_ne!(a, b);
        assert!(a.to_str().unwrap().ends_with(".cozo"));
    }
}
