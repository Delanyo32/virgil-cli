use std::collections::{HashMap, HashSet};

use crate::language::Language;
use crate::models::{ImportInfo, SymbolKind};

/// A node in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GraphNode {
    /// File-level node (most languages)
    File(String),
    /// Package/directory-level node (Go only)
    Package(String),
}

/// A lightweight representation of an exported symbol for cross-file matching.
#[derive(Debug, Clone)]
pub struct ExportedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub start_line: u32,
}

/// Per-file extracted metadata stored in the index.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub language: Language,
    pub line_count: u32,
    pub symbol_count: usize,
    pub exported_symbols: Vec<ExportedSymbol>,
    pub imports: Vec<ImportInfo>,
}

/// Cross-file index built during the pre-pass.
/// Contains per-file metadata and a resolved dependency graph.
pub struct ProjectIndex {
    pub files: HashMap<String, FileEntry>,
    /// Resolved dependency graph: source node -> set of target nodes
    pub edges: HashMap<GraphNode, HashSet<GraphNode>>,
    /// All file paths in the workspace, for fast existence probing by resolvers
    pub known_files: HashSet<String>,
}

impl Default for ProjectIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectIndex {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            edges: HashMap::new(),
            known_files: HashSet::new(),
        }
    }

    /// Build reverse edges: for each (A -> B), produce (B -> A).
    /// Used for afferent coupling (fan-in) analysis.
    pub fn reverse_edges(&self) -> HashMap<GraphNode, HashSet<GraphNode>> {
        let mut reverse: HashMap<GraphNode, HashSet<GraphNode>> = HashMap::new();
        for (from, tos) in &self.edges {
            for to in tos {
                reverse
                    .entry(to.clone())
                    .or_default()
                    .insert(from.clone());
            }
        }
        reverse
    }
}

impl GraphNode {
    pub fn path(&self) -> &str {
        match self {
            GraphNode::File(p) | GraphNode::Package(p) => p,
        }
    }
}
