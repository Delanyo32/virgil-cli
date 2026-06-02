//! Wrapper around a single DuckDB connection. Owns lifecycle, applies
//! schema, loads duckpgq, version-checks on reopen.
//!
//! Mirrors `src/cozo/store.rs`. Mutability is unified — DuckDB doesn't
//! distinguish at the connection level, but we keep `run_script` /
//! `run_query` as parallel names to keep callsite porting mechanical.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use duckdb::Connection;
use duckdb::types::Value;

use super::SCHEMA_VERSION;
use super::schema;

/// A DuckDB database handle.
pub struct DbStore {
    conn: Mutex<Connection>,
    /// `true` when the schema was applied fresh during this open (i.e.
    /// the caller must populate the store from a workspace before
    /// querying).
    fresh: bool,
}

impl DbStore {
    /// Open a fresh in-memory store with the schema + duckpgq applied.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| anyhow!("failed to open duckdb mem store: {e}"))?;
        let store = Self {
            conn: Mutex::new(conn),
            fresh: true,
        };
        store.load_duckpgq()?;
        store.apply_schema()?;
        store.record_schema_version()?;
        Ok(store)
    }

    /// Open a file-backed DuckDB store at `path`. Wipe-and-recreate
    /// on schema-version mismatch.
    pub fn open_persistent(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating cache dir {}", parent.display()))?;
        }

        if path.exists() {
            match Self::try_reopen(path)? {
                Some(store) => return Ok(store),
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }

        let conn = Connection::open(path)
            .map_err(|e| anyhow!("failed to open duckdb store at {}: {e}", path.display()))?;
        let store = Self {
            conn: Mutex::new(conn),
            fresh: true,
        };
        store.load_duckpgq()?;
        store.apply_schema()?;
        store.record_schema_version()?;
        Ok(store)
    }

    fn try_reopen(path: &Path) -> Result<Option<Self>> {
        if path.is_dir() {
            return Ok(None);
        }
        let conn = match Connection::open(path) {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        let store = Self {
            conn: Mutex::new(conn),
            fresh: false,
        };
        if store.load_duckpgq().is_err() {
            return Ok(None);
        }
        let matches = store.schema_version_matches();
        if matches { Ok(Some(store)) } else { Ok(None) }
    }

    fn schema_version_matches(&self) -> bool {
        let conn = self.conn.lock().unwrap();
        let stmt = conn.prepare("SELECT value FROM build_meta WHERE key = 'schema_version'");
        match stmt {
            Ok(mut s) => match s.query_row([], |r| r.get::<_, String>(0)) {
                Ok(v) => v == SCHEMA_VERSION.to_string(),
                Err(_) => false,
            },
            Err(_) => false,
        }
    }

    pub fn fresh(&self) -> bool {
        self.fresh
    }

    /// Open a new sibling connection to the already-opened database via
    /// `Connection::try_clone`. Used by serve mode to build a pool of
    /// read connections, one per concurrent query worker — DuckDB
    /// supports concurrent reads across sibling connections (MVCC).
    ///
    /// duckpgq is loaded per-connection, so the clone re-`LOAD`s it. The
    /// clone is never `fresh` (the schema already exists on the shared
    /// database).
    pub fn try_clone_store(&self) -> Result<Self> {
        let conn = self.conn.lock().unwrap();
        let cloned = conn
            .try_clone()
            .map_err(|e| anyhow!("failed to clone duckdb connection: {e}"))?;
        drop(conn);
        let store = Self {
            conn: Mutex::new(cloned),
            fresh: false,
        };
        store.load_duckpgq()?;
        Ok(store)
    }

    fn load_duckpgq(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // INSTALL writes to the shared ~/.duckdb/extensions/ cache;
        // cargo runs lib tests in parallel, so multiple connections
        // can race the same INSTALL and either tread on each other's
        // temp file or LOAD before the rename completes. Serialise
        // INSTALL across the process once, then LOAD per-connection.
        ensure_duckpgq_installed(&conn)?;
        conn.execute_batch("LOAD duckpgq;")
            .map_err(|e| anyhow!("LOAD duckpgq failed: {e}"))?;
        Ok(())
    }

    fn apply_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        for stmt in schema::create_statements() {
            conn.execute(stmt, [])
                .map_err(|e| anyhow!("applying CREATE TABLE: {e}\nstmt: {stmt}"))?;
        }
        for stmt in schema::index_statements() {
            conn.execute(stmt, [])
                .map_err(|e| anyhow!("applying CREATE INDEX: {e}\nstmt: {stmt}"))?;
        }
        for stmt in schema::pgq_statements() {
            conn.execute(stmt, [])
                .map_err(|e| anyhow!("applying CREATE PROPERTY GRAPH: {e}\nstmt: {stmt}"))?;
        }
        Ok(())
    }

    fn record_schema_version(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO build_meta(key, value) VALUES ('schema_version', '{}')",
                SCHEMA_VERSION
            ),
            [],
        )
        .map_err(|e| anyhow!("recording schema_version: {e}"))?;
        Ok(())
    }

    /// Mutating query helper. `params` substitute `$name` placeholders
    /// as SQL literals (the values come from trusted `--param` CLI
    /// input).
    pub fn run_script(&self, sql: &str, params: BTreeMap<String, Value>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let stripped = strip_sql_comments(sql);
        let inlined = inline_named_params(&stripped, &params);
        conn.execute_batch(&inlined)
            .map_err(|e| anyhow!("duckdb script failed: {e}\nsql: {inlined}"))?;
        Ok(())
    }

    /// Read-only query helper. Returns rows as `Vec<Vec<Value>>`.
    ///
    /// Column headers are read from the first row's `as_ref()`
    /// statement after `query()` materialises the result set. Calling
    /// `column_count` on a prepared-but-not-yet-queried statement
    /// panics in duckdb 1.2 — the schema isn't bound until execution.
    pub fn run_query(&self, sql: &str, params: BTreeMap<String, Value>) -> Result<QueryRows> {
        let conn = self.conn.lock().unwrap();
        let stripped = strip_sql_comments(sql);
        let inlined = inline_named_params(&stripped, &params);
        let mut stmt = conn
            .prepare(&inlined)
            .map_err(|e| anyhow!("duckdb prepare failed: {e}\nsql: {inlined}"))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| anyhow!("duckdb query failed: {e}\nsql: {inlined}"))?;
        let mut out: Vec<Vec<Value>> = Vec::new();
        let mut headers: Vec<String> = Vec::new();
        while let Some(row) = rows.next().map_err(|e| anyhow!("duckdb row: {e}"))? {
            if headers.is_empty() {
                // Snapshot column names from the underlying statement
                // now that it has a bound result set.
                let stmt_ref = row.as_ref();
                let n = stmt_ref.column_count();
                headers = (0..n)
                    .map(|i| stmt_ref.column_name(i).cloned().unwrap_or_default())
                    .collect();
            }
            let mut r = Vec::with_capacity(headers.len());
            for i in 0..headers.len() {
                let v: Value = row.get(i).map_err(|e| anyhow!("duckdb get col {i}: {e}"))?;
                r.push(v);
            }
            out.push(r);
        }
        Ok(QueryRows { headers, rows: out })
    }

    /// Borrow the underlying connection. Used by [`DbWriter`] for Arrow
    /// ingest paths that need typed access.
    pub fn with_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Connection) -> R,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }
}

/// Rows returned by [`DbStore::run_query`]. Shape mirrors
/// `cozo::NamedRows` so the query runner can stay engine-agnostic.
#[derive(Debug)]
pub struct QueryRows {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Strip SQL comments (`-- to end of line` and `/* ... */` blocks) so
/// they (a) don't get scanned for `$name` placeholders we'd
/// mistakenly bind, and (b) don't trip up duckpgq's parser, which
/// rejects `--` in front of `GRAPH_TABLE` statements where plain
/// DuckDB accepts them.
///
/// Stays inside string literals — both `'...'` and `"..."` — without
/// stripping their contents.
fn strip_sql_comments(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // String literal: copy until closing quote, handling doubled
        // quotes as escapes (SQL convention).
        if b == b'\'' || b == b'"' {
            let q = b;
            out.push(b as char);
            i += 1;
            while i < bytes.len() {
                if bytes[i] == q {
                    out.push(q as char);
                    i += 1;
                    if i < bytes.len() && bytes[i] == q {
                        // doubled quote = escape
                        out.push(q as char);
                        i += 1;
                        continue;
                    }
                    break;
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }
        // Line comment: drop everything to end of line (keep the \n).
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment: drop until */.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // Pass through one char (handle multi-byte UTF-8 correctly).
        let ch = sql[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Substitute `$name` placeholders with literal SQL values.
///
/// We don't use DuckDB prepared-statement binding because duckpgq's
/// `GRAPH_TABLE(... MATCH ... WHERE ?)` rejects positional placeholders
/// — the WHERE clause is interpreted by the PGQ engine, not by the
/// outer SQL parameter binder. Values come from `--param k=v` CLI
/// input which is trusted (the user runs their own queries on their
/// own machine); injection isn't a threat model.
///
/// Stays inside string literals so a `$name` inside `'...'` is left
/// alone.
fn inline_named_params(sql: &str, params: &BTreeMap<String, Value>) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let q = b;
            out.push(q as char);
            i += 1;
            while i < bytes.len() {
                if bytes[i] == q {
                    out.push(q as char);
                    i += 1;
                    if i < bytes.len() && bytes[i] == q {
                        out.push(q as char);
                        i += 1;
                        continue;
                    }
                    break;
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }
        if b == b'$'
            && i + 1 < bytes.len()
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let name = &sql[i + 1..j];
            match params.get(name) {
                Some(v) => out.push_str(&format_sql_literal(v)),
                None => out.push_str(&sql[i..j]),
            }
            i = j;
            continue;
        }
        let ch = sql[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn format_sql_literal(v: &Value) -> String {
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
        Value::UTinyInt(n) => n.to_string(),
        Value::USmallInt(n) => n.to_string(),
        Value::UInt(n) => n.to_string(),
        Value::UBigInt(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Double(n) => n.to_string(),
        Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
        // Catch-all: format via Debug, wrap in quotes. Templates that
        // need exotic types should bind them via plain SQL literals
        // directly rather than --param.
        other => format!("'{:?}'", other).replace('\'', "''"),
    }
}

/// Run `INSTALL duckpgq FROM community` at most once per process,
/// serialised across threads. The shared `~/.duckdb/extensions/`
/// cache isn't safe for concurrent writes from a single process —
/// CI surfaced this as a flaky "Extension not found" from LOAD when
/// cargo runs lib tests in parallel and several DbStore opens race
/// the install at the same time.
fn ensure_duckpgq_installed(conn: &Connection) -> Result<()> {
    use std::sync::Mutex;
    static INSTALLED: Mutex<bool> = Mutex::new(false);
    let mut done = INSTALLED.lock().unwrap();
    if *done {
        return Ok(());
    }
    let _ = conn.execute_batch("SET allow_community_extensions = true;");
    conn.execute_batch("INSTALL duckpgq FROM community;")
        .map_err(|e| anyhow!("INSTALL duckpgq FROM community failed: {e}"))?;
    *done = true;
    Ok(())
}

/// Cache file path for a workspace identified by `id`. Returns
/// `~/.cache/virgil/<hash>.duckdb`.
pub fn cache_dir_for_db(id: &str) -> Result<PathBuf> {
    let base = dirs::cache_dir()
        .context("could not determine OS cache directory")?
        .join("virgil");
    let hash = stable_hash(id);
    Ok(base.join(format!("{hash:016x}.duckdb")))
}

fn stable_hash(s: &str) -> u64 {
    // FNV-1a 64. Same hash function as cozo::cache_dir_for so the
    // cache_dir_for("foo") path differs from cache_dir_for_db("foo")
    // only by file extension.
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
    fn opens_in_memory_and_records_schema_version() {
        let store = DbStore::open_in_memory().expect("open");
        assert!(store.fresh());
        let rows = store
            .run_query(
                "SELECT value FROM build_meta WHERE key = 'schema_version'",
                BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0], Value::Text(SCHEMA_VERSION.to_string()));
    }

    #[test]
    fn persistent_store_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.duckdb");

        let store = DbStore::open_persistent(&path).expect("open fresh");
        assert!(store.fresh());
        store
            .run_script(
                "INSERT INTO symbol VALUES \
                 ('a.ts|1|0|login|function', 'function', 'login', 'login', \
                  'typescript', 'public', 'a.ts', NULL, \
                  false, false, false, false, true)",
                BTreeMap::new(),
            )
            .expect("insert");
        drop(store);

        let store = DbStore::open_persistent(&path).expect("reopen");
        assert!(!store.fresh(), "expected warm reopen");
        let rows = store
            .run_query("SELECT name FROM symbol", BTreeMap::new())
            .expect("query");
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0], Value::Text("login".into()));
    }

    #[test]
    fn pgq_match_walks_call_edges() {
        // Seeds three symbols + two call_edges (a→b→c) and asks PGQ
        // for everyone a transitive caller of `c` reaches. This is the
        // shape `find_callers` will run, but minimal — proves that
        // duckpgq parses our DDL and walks our edges correctly.
        let store = DbStore::open_in_memory().expect("open");
        store
            .run_script(
                "INSERT INTO symbol VALUES \
                   ('a', 'function', 'a', 'a', 'rust', 'public', 'lib.rs', NULL, \
                    false, false, false, false, true), \
                   ('b', 'function', 'b', 'b', 'rust', 'public', 'lib.rs', NULL, \
                    false, false, false, false, true), \
                   ('c', 'function', 'c', 'c', 'rust', 'public', 'lib.rs', NULL, \
                    false, false, false, false, true)",
                BTreeMap::new(),
            )
            .expect("insert symbols");
        store
            .run_script(
                "INSERT INTO call_edge VALUES ('a','b','lib.rs'), ('b','c','lib.rs')",
                BTreeMap::new(),
            )
            .expect("insert edges");
        let rows = store
            .run_query(
                "SELECT caller_name FROM GRAPH_TABLE (codegraph \
                   MATCH ANY ACYCLIC (a:symbol)-[e:calls]->+(c:symbol) \
                   WHERE c.id = 'c' \
                   COLUMNS (a.name AS caller_name) \
                 ) ORDER BY caller_name",
                BTreeMap::new(),
            )
            .expect("pgq match");
        // Transitive walk to c reaches {a, b}; c itself depends on
        // whether `->*` is zero-or-more or one-or-more in duckpgq.
        let names: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|r| match &r[0] {
                Value::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"a".to_string()), "expected a in {names:?}");
        assert!(names.contains(&"b".to_string()), "expected b in {names:?}");
    }

    #[test]
    fn duckdb_handles_leading_sql_comments() {
        let store = DbStore::open_in_memory().expect("open");
        let res = store.run_query("-- a comment\nSELECT 1 AS x", BTreeMap::new());
        assert!(res.is_ok(), "prepare with leading comment: {res:?}");
    }

    #[test]
    fn all_builtin_templates_parse_against_empty_store() {
        // Smoke-check that each .sql template under
        // src/queries/builtin/ at least parses cleanly through DuckDB
        // (and through duckpgq for the PGQ-flavored ones). Runs each
        // template against an empty store with a dummy $name binding;
        // expects zero rows but no error. Reports every failure so we
        // see the full picture in one run.
        let store = DbStore::open_in_memory().expect("open");
        let templates_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/queries/builtin");
        let mut paths: Vec<_> = std::fs::read_dir(&templates_dir)
            .expect("read templates dir")
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sql"))
            .collect();
        paths.sort();
        assert_eq!(paths.len(), 7, "expected 7 .sql templates");
        let mut failures = Vec::new();
        for path in &paths {
            let sql = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let mut params = BTreeMap::new();
            params.insert("name".to_string(), Value::Text("__dummy__".to_string()));
            if let Err(e) = store.run_query(&sql, params) {
                failures.push(format!(
                    "  {} -> {e}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
        if !failures.is_empty() {
            panic!(
                "{} template(s) failed:\n{}",
                failures.len(),
                failures.join("\n")
            );
        }
    }

    #[test]
    fn cloned_store_reads_shared_database() {
        // try_clone_store yields an independent connection that sees the
        // same data and can run a PGQ query (proves duckpgq loaded on the
        // clone). Mirrors how serve mode hands one clone per worker.
        let store = DbStore::open_in_memory().expect("open");
        store
            .run_script(
                "INSERT INTO symbol VALUES \
                   ('a', 'function', 'a', 'a', 'rust', 'public', 'lib.rs', NULL, \
                    false, false, false, false, true), \
                   ('b', 'function', 'b', 'b', 'rust', 'public', 'lib.rs', NULL, \
                    false, false, false, false, true)",
                BTreeMap::new(),
            )
            .expect("insert");
        store
            .run_script(
                "INSERT INTO call_edge VALUES ('a','b','lib.rs')",
                BTreeMap::new(),
            )
            .expect("edge");

        let clone = store.try_clone_store().expect("clone");
        assert!(!clone.fresh());
        let rows = clone
            .run_query(
                "SELECT caller_name FROM GRAPH_TABLE (codegraph \
                   MATCH ANY ACYCLIC (a:symbol)-[e:calls]->+(c:symbol) \
                   WHERE c.id = 'b' \
                   COLUMNS (a.name AS caller_name))",
                BTreeMap::new(),
            )
            .expect("pgq on clone");
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0], Value::Text("a".into()));
    }

    #[test]
    fn cache_dir_is_stable_and_distinct() {
        let a = cache_dir_for_db("project-a").unwrap();
        let a_again = cache_dir_for_db("project-a").unwrap();
        let b = cache_dir_for_db("project-b").unwrap();
        assert_eq!(a, a_again);
        assert_ne!(a, b);
        assert!(a.to_str().unwrap().ends_with(".duckdb"));
    }
}
