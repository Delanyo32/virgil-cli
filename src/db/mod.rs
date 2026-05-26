//! DuckDB-backed fact store — experimental swap for the Cozo backend.
//!
//! See `docs/experiments/duckdb-swap.md` for the locked plan. This module
//! mirrors `src/cozo/`'s shape: schema DDL, a store wrapper, a batched
//! writer, the populate tail, and a queries helper. There is no
//! `incremental` module (deferred — cold + warm only).

pub mod from_code_graph;
pub mod queries;
pub mod schema;
pub mod store;
pub mod writer;

pub use from_code_graph::populate;
pub use store::{DbStore, cache_dir_for_db};
pub use writer::DbWriter;

/// Bump when the schema in [`schema`] changes shape in a way that
/// requires a fresh build. Persisted into `build_meta(schema_version)`
/// and checked on open; mismatch wipes the file.
///
/// 1: initial DuckDB schema (Cozo schema v9 ported 1:1 to DuckDB tables
/// + a `CREATE PROPERTY GRAPH codegraph` for duckpgq).
pub const SCHEMA_VERSION: u32 = 1;
