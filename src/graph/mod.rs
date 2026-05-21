pub mod builder;
pub mod intern;
pub mod metrics;

use std::collections::HashMap;

pub use intern::{Spur, Symbols};

use crate::language::Language;
use crate::models::{
    AttrsBucket, FieldTypeRow, ImportInfo, InheritanceRow, ParameterTypeRow, ReferencesBucket,
    ReturnsTypeRow, SymbolKind, SymbolVisibility, ThrowsRow, TypeRow,
};

/// Stable index into [`CodeGraph::nodes`]. Replaces `petgraph::NodeIndex`.
pub type NodeIndex = usize;

/// A node in the import resolution result. Most languages resolve to a file;
/// Go resolves to a package directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GraphNode {
    File(String),
    Package(String),
}

#[derive(Debug, Clone)]
pub enum NodeWeight {
    File {
        path: Spur,
        language: Language,
    },
    Symbol {
        name: Spur,
        /// Scope-qualified name. Computed in `absorb_file_data` by walking
        /// the chain of containing symbols and joining their names with the
        /// language-specific separator. Top-level symbols have
        /// `qualified_name == name`.
        qualified_name: Spur,
        kind: SymbolKind,
        file_path: Spur,
        start_byte: u32,
        end_byte: u32,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        exported: bool,
        visibility: SymbolVisibility,
        is_async: bool,
        is_static: bool,
        is_abstract: bool,
        is_mutable: bool,
    },
    CallSite {
        name: Spur,
        file_path: Spur,
        line: u32,
        start_byte: u32,
        end_byte: u32,
        /// Literal arguments at this call site (strings/numbers/bools only).
        /// `None` for the common case of a call with no literal arguments —
        /// avoids the 24-byte `Vec` header on every CallSite.
        arg_literals: Option<Box<[Spur]>>,
        /// Name of the enclosing test function, when this call site sits
        /// inside a test (path matches `is_test_file` and the enclosing
        /// symbol's name follows a test naming convention).
        enclosing_test_name: Option<Spur>,
        /// The Symbol node that contains this call site, if any.
        caller_symbol: Option<NodeIndex>,
    },
}

#[derive(Debug, Clone)]
pub enum EdgeWeight {
    DefinedIn,
    Calls,
    Imports,
    Exports,
    Contains,
}

/// In-memory build-time graph. Lives for the duration of a cold or
/// incremental build, then gets walked once by `cozo::populate` and
/// dropped. Replaces the old `petgraph::DiGraph` with a flat
/// adjacency-list — same operations, no external dependency.
pub struct CodeGraph {
    pub nodes: Vec<NodeWeight>,
    /// Edges grouped by source node id. `out_edges[u]` lists every
    /// outbound `(target, weight)` from node `u`.
    pub out_edges: Vec<Vec<(NodeIndex, EdgeWeight)>>,
    /// Edges grouped by target node id. Mirrors `out_edges` for
    /// constant-time incoming-edge lookup.
    pub in_edges: Vec<Vec<(NodeIndex, EdgeWeight)>>,
    /// Shared string interner. Every interned `Spur` (file path, symbol name)
    /// is resolvable through this handle.
    pub symbols: Symbols,
    /// file_path (interned) -> NodeIndex
    pub file_nodes: HashMap<Spur, NodeIndex>,
    /// (file_path, start_line) -> NodeIndex
    pub symbol_nodes: HashMap<(Spur, u32), NodeIndex>,
    /// symbol name (interned) -> list of NodeIndex for all symbols with that name
    pub symbols_by_name: HashMap<Spur, Vec<NodeIndex>>,
    /// Raw import-info per file, preserved so the Cozo writer can persist
    /// pre-resolution import data into `raw_import` for the incremental
    /// refresh path (issue 08). Keyed by source file path.
    pub raw_imports: HashMap<String, Vec<ImportInfo>>,
    /// Extracted comments per file. Populated by the builder when
    /// comment queries succeed. Keyed by source file path. Empty for
    /// languages whose extractor doesn't emit comments yet.
    pub comments: HashMap<String, Vec<crate::models::CommentInfo>>,
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
    /// Per-file per-language attribute buckets (issue #15). Only the
    /// file's source language is populated.
    pub attrs: HashMap<String, AttrsBucket>,
    /// Per-file occurrence/scope/binding facts (issue #16). The
    /// Cozoscript resolver consumes these to materialise `references`.
    pub references: HashMap<String, ReferencesBucket>,
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            out_edges: Vec::new(),
            in_edges: Vec::new(),
            symbols: Symbols::new(),
            file_nodes: HashMap::new(),
            symbol_nodes: HashMap::new(),
            symbols_by_name: HashMap::new(),
            raw_imports: HashMap::new(),
            comments: HashMap::new(),
            types: HashMap::new(),
            param_types: HashMap::new(),
            returns_types: HashMap::new(),
            inheritance: HashMap::new(),
            field_types: HashMap::new(),
            throws: HashMap::new(),
            attrs: HashMap::new(),
            references: HashMap::new(),
        }
    }

    /// Allocate a fresh node and return its index. O(1) amortised.
    pub fn add_node(&mut self, weight: NodeWeight) -> NodeIndex {
        let id = self.nodes.len();
        self.nodes.push(weight);
        self.out_edges.push(Vec::new());
        self.in_edges.push(Vec::new());
        id
    }

    /// Record a directed edge `from -> to` carrying `weight`. The same
    /// weight is stored in both adjacency lists so traversals can read
    /// edge metadata regardless of direction.
    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, weight: EdgeWeight) {
        self.out_edges[from].push((to, weight.clone()));
        self.in_edges[to].push((from, weight));
    }

    /// Borrow the weight of `idx`, or `None` if out of range.
    pub fn node_weight(&self, idx: NodeIndex) -> Option<&NodeWeight> {
        self.nodes.get(idx)
    }

    /// Iterator over every valid node index.
    pub fn node_indices(&self) -> std::ops::Range<NodeIndex> {
        0..self.nodes.len()
    }

    /// Find a symbol node by file path and start line.
    pub fn find_symbol(&self, file_path: &str, start_line: u32) -> Option<NodeIndex> {
        let spur = self.symbols.get(file_path)?;
        self.symbol_nodes.get(&(spur, start_line)).copied()
    }

    /// Find all symbol nodes with a given name.
    pub fn find_symbols_by_name(&self, name: &str) -> &[NodeIndex] {
        let Some(spur) = self.symbols.get(name) else {
            return &[];
        };
        self.symbols_by_name
            .get(&spur)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}
