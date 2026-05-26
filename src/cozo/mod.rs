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
/// relations renamed, `field_type`/`type`/`extends`/`implements`/
/// `throws`/`comment` relations added, per-language `*_attrs` tables
/// declared (empty until Phase 4).
///
/// 4: Issue #16 — `occurrence`, `scope`, `binding` fact-emission
/// relations added per ADR-0005.
///
/// 5: Added `imports:by_importer {importer_file_id}` index.
///
/// 6: Removed the `references` relation and the eager Cozoscript
/// resolver that materialised it. The raw `occurrence`/`scope`/
/// `binding` facts stay — callers needing resolved references run
/// their own Cozoscript over those at query time.
///
/// 7: Eager call *resolution* disabled (`RESOLVE_CALLS_EAGERLY = false`
/// in `src/graph/builder.rs`). `*calls` relation still exists in the
/// schema but stays empty after build. Eager import resolution kept
/// on; `*imports` rows still materialised. Motivated by the openclaw
/// container OOM (3.26 GiB peak under a 4 GiB cap, dropped to ~800 MiB
/// after the change).
///
/// 8: Adds `call_site` relation. Holds the raw per-call-site facts
/// `extract_call_sites` produces (caller_id?, callee_name, file_path,
/// byte range) without doing any cross-file resolution at build. The
/// `find_callers` / `find_callees` / `find_cycles` templates and the
/// example `calls_at_query_time.cozoql` join `*call_site` to
/// `*symbol` + `*imports` to derive resolved call edges at query
/// time. Restores the accuracy of the pre-v7 `*calls` resolver
/// (including method calls — the references extractor's narrow
/// `occurrence_kind: 'call'` tags only bare-identifier calls) without
/// paying the OOM-prone build-time resolution scratch.
///
/// 9: Added `call_edge {caller_id, callee_id => file_path}` relation
/// populated at build time by `from_code_graph::resolve_and_emit_call_edges`,
/// plus `symbol:by_name_kind {name, kind}` index. Lets queries that need
/// resolved call edges skip the per-query recursion.
pub const SCHEMA_VERSION: u32 = 9;
