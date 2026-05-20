//! Cozo-backed fact store for the graph migration (issue 02).
//!
//! At this stage the module is intentionally minimal — it stands up a
//! `CozoStore`, declares the cross-function graph schema, and provides a
//! batched writer the absorber can call into. CFG facts and metric facts
//! land in later issues (03 and 04 respectively).

pub mod from_code_graph;
pub mod incremental;
pub mod queries;
pub mod schema;
pub mod store;
pub mod writer;

pub use from_code_graph::{is_warm_compatible, populate, wipe_workspace_relations};
pub use incremental::{
    WorkspaceDiff, delete_file_facts, incremental_refresh, resolve_cross_file_edges, workspace_diff,
};
pub use store::{CozoStore, cache_dir_for};
pub use writer::CozoWriter;

/// Bump when the schema in [`schema`] changes in a way that requires a
/// rebuild from scratch. Persisted into `build_meta` and checked on open
/// so a mismatch wipes the old store and triggers a clean repopulate.
///
/// 3: Datalog-model migration (Phase 1). Symbol IDs become String, edge
/// relations renamed, `references` shape changed (match_index key,
/// nullable `referent_id`), `field_type`/`type`/`extends`/`implements`/
/// `throws`/`comment` relations added, per-language `*_attrs` tables
/// declared (empty until Phase 4).
pub const SCHEMA_VERSION: u32 = 3;
