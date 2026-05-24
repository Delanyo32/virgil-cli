pub mod builder;
pub mod intern;
pub mod metrics;

use std::collections::HashMap;

pub use intern::{Spur, Symbols};

use crate::models::{
    CommentInfo, FieldTypeRow, InheritanceRow, ParameterTypeRow, ReturnsTypeRow, ThrowsRow, TypeRow,
};

/// A node in the import resolution result. Most languages resolve to a file;
/// Go resolves to a package directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GraphNode {
    File(String),
    Package(String),
}

/// Build-time, in-process scratch state that survives only for the
/// duration of a cold/incremental build. After Slice B the per-node
/// adjacency-list graph is gone — `Symbol`/`File`/`CallSite` rows are
/// streamed to Cozo during absorb, and the cross-file `Imports`/`Calls`
/// edges that used to live on `out_edges` are emitted directly to Cozo
/// rows by the deferred-resolution loop in `builder.rs`.
///
/// What's still here serves the populate phases that need cross-file
/// symbol lookup against the workspace symbol table — comments and
/// types/inheritance both resolve `(file_path, name) -> symbol_id`
/// against this map instead of re-walking every symbol.
pub struct CodeGraph {
    /// Shared string interner. Kept because per-file local lookups
    /// (file_symbols_by_name etc. inside `GraphBuilder::build`) still
    /// key on `Spur` to avoid String allocations during absorb.
    pub symbols: Symbols,
    /// `(file_path, name) -> [symbol_id]`. Populated incrementally
    /// during absorb. Multiple ids per key cover same-name overloads /
    /// shadowing. The emit-comments and emit-types phases read this
    /// instead of walking a `nodes` Vec.
    pub symbol_ids_by_name: HashMap<(String, String), Vec<String>>,
    /// Extracted comments per file. Populated by the builder when
    /// comment queries succeed. Keyed by source file path. Empty for
    /// languages whose extractor doesn't emit comments yet.
    pub comments: HashMap<String, Vec<CommentInfo>>,
    /// Per-file type-expression rows (issue #13). One row per unique
    /// `(file_path, display_name)`; the emitter dedups + assigns
    /// `type.id`.
    pub types: HashMap<String, Vec<TypeRow>>,
    /// Per-file parameter→type bindings (issue #13). The emitter joins
    /// these to `types` by `display_name` to populate `parameter.type_id`.
    pub param_types: HashMap<String, Vec<ParameterTypeRow>>,
    /// Per-file function→return-type bindings (issue #13).
    pub returns_types: HashMap<String, Vec<ReturnsTypeRow>>,
    /// Per-file class/trait inheritance edges (issue #13). The emitter
    /// resolves both endpoints to symbol IDs where possible and writes
    /// `extends` or `implements` rows.
    pub inheritance: HashMap<String, Vec<InheritanceRow>>,
    /// Per-file typed-field bindings (issue #14). One row per typed
    /// struct/class field; untyped fields produce no entry.
    pub field_types: HashMap<String, Vec<FieldTypeRow>>,
    /// Per-file `throws` rows (issue #13 followup). Populated only by
    /// Java/C#/PHP extractors; other languages leave this map empty.
    pub throws: HashMap<String, Vec<ThrowsRow>>,
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            symbols: Symbols::new(),
            symbol_ids_by_name: HashMap::new(),
            comments: HashMap::new(),
            types: HashMap::new(),
            param_types: HashMap::new(),
            returns_types: HashMap::new(),
            inheritance: HashMap::new(),
            field_types: HashMap::new(),
            throws: HashMap::new(),
        }
    }
}
