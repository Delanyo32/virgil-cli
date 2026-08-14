//! DuckDB-backed fact store.
//!
//! Schema DDL, a store wrapper, a batched writer, and the populate tail.
//! Cold + warm builds only — incremental refresh is not implemented.

pub mod from_code_graph;
pub mod schema;
pub mod store;
pub mod writer;

pub use from_code_graph::populate;
pub use store::{DbStore, cache_dir_for_db, cache_root};
pub use writer::DbWriter;

/// Bump when the schema in [`schema`] changes shape in a way that
/// requires a fresh build. Persisted into `build_meta(schema_version)`
/// and checked on open; mismatch wipes the file.
///
/// - 1: initial DuckDB schema (ported from the prior Cozo store).
/// - 2: add `call_site.receiver` (immediate object/namespace of a call).
/// - 3: `scope.kind` for body blocks now holds the owning tree-sitter
///   construct (for_statement, if_statement, …) instead of generic "block".
/// - 4: add `local_type` (local variable -> declared/inferred type name)
///   for type-aware call resolution.
/// - 5: drop the never-populated `calls` and `nolint` tables. Resolved
///   call edges live in `call_edge`.
/// - 6: add `file.content` (full source text) so the store is
///   self-contained and readers never touch the filesystem.
pub const SCHEMA_VERSION: u32 = 6;
