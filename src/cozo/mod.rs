//! Cozo-backed fact store for the graph migration (issue 02).
//!
//! At this stage the module is intentionally minimal — it stands up a
//! `CozoStore`, declares the cross-function graph schema, and provides a
//! batched writer the absorber can call into. CFG facts and metric facts
//! land in later issues (03 and 04 respectively).

pub mod from_code_graph;
pub mod queries;
pub mod schema;
pub mod store;
pub mod writer;

pub use from_code_graph::{is_warm_compatible, populate, wipe_workspace_relations};
pub use store::{CozoStore, cache_dir_for};
pub use writer::CozoWriter;

/// Bump when the schema in [`schema`] changes in a way that requires a
/// rebuild from scratch. Persisted into `build_meta` so future issue 07
/// can detect mismatches.
pub const SCHEMA_VERSION: u32 = 1;
