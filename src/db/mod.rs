//! DuckDB-backed fact store.
//!
//! Schema DDL, a store wrapper, a batched writer, and the populate tail.
//! Cold + warm builds only — incremental refresh is not implemented.
//! See `docs/experiments/duckdb-swap.md` for the design.

pub mod from_code_graph;
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
/// - 1: initial DuckDB schema (ported from the prior Cozo store
///   + a `CREATE PROPERTY GRAPH codegraph` for duckpgq).
/// - 2: add `call_site.receiver` (immediate object/namespace of a call).
/// - 3: `scope.kind` for body blocks now holds the owning tree-sitter
///   construct (for_statement, if_statement, …) instead of generic "block".
/// - 4: add `local_type` (local variable -> declared/inferred type name)
///   for type-aware call resolution.
/// - 5: drop the never-populated `calls` and `nolint` tables. Resolved
///   call edges live in `call_edge`.
pub const SCHEMA_VERSION: u32 = 5;
