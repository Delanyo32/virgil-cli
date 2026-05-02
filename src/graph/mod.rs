pub mod builder;
pub mod cfg;
pub mod metrics;
pub mod resource;
pub mod taint;

use std::collections::HashMap;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::language::Language;
use crate::models::SymbolKind;

/// A node in the import resolution result. Most languages resolve to a file;
/// Go resolves to a package directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GraphNode {
    File(String),
    Package(String),
}

#[derive(Debug, Clone)]
pub enum SourceKind {
    UserInput,
    DatabaseRead,
    FileRead,
    EnvironmentVar,
    NetworkRead,
    Deserialization,
}

#[derive(Debug, Clone)]
pub enum NodeWeight {
    File {
        path: String,
        language: Language,
    },
    Symbol {
        name: String,
        kind: SymbolKind,
        file_path: String,
        start_line: u32,
        end_line: u32,
        exported: bool,
    },
    CallSite {
        name: String,
        file_path: String,
        line: u32,
    },
    Parameter {
        name: String,
        function_node: NodeIndex,
        position: usize,
        is_taint_source: bool,
    },
    ExternalSource {
        kind: SourceKind,
        file_path: String,
        line: u32,
    },
}

#[derive(Debug, Clone)]
pub enum EdgeWeight {
    DefinedIn,
    Calls,
    Imports,
    FlowsTo,
    SanitizedBy { sanitizer: String },
    Exports,
    Acquires { resource_type: String },
    ReleasedBy,
    Contains,
}

pub struct CodeGraph {
    pub graph: DiGraph<NodeWeight, EdgeWeight>,
    /// file_path -> NodeIndex
    pub file_nodes: HashMap<String, NodeIndex>,
    /// (file_path, start_line) -> NodeIndex
    pub symbol_nodes: HashMap<(String, u32), NodeIndex>,
    /// symbol name -> list of NodeIndex for all symbols with that name
    pub symbols_by_name: HashMap<String, Vec<NodeIndex>>,
    /// function NodeIndex -> its CFG
    pub function_cfgs: HashMap<NodeIndex, cfg::FunctionCfg>,
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            file_nodes: HashMap::new(),
            symbol_nodes: HashMap::new(),
            symbols_by_name: HashMap::new(),
            function_cfgs: HashMap::new(),
        }
    }

    // --- Call graph traversal methods (Phase 3) ---

    /// BFS traversal to find callees of seed symbols up to max_depth.
    pub fn traverse_callees(&self, seeds: &[NodeIndex], max_depth: usize) -> Vec<NodeIndex> {
        self.traverse_calls(seeds, max_depth, Direction::Outgoing)
    }

    /// BFS traversal to find callers of seed symbols up to max_depth.
    pub fn traverse_callers(&self, seeds: &[NodeIndex], max_depth: usize) -> Vec<NodeIndex> {
        self.traverse_calls(seeds, max_depth, Direction::Incoming)
    }

    fn traverse_calls(
        &self,
        seeds: &[NodeIndex],
        max_depth: usize,
        direction: Direction,
    ) -> Vec<NodeIndex> {
        use std::collections::{HashSet, VecDeque};

        let mut visited: HashSet<NodeIndex> = seeds.iter().copied().collect();
        let mut queue: VecDeque<(NodeIndex, usize)> = seeds.iter().map(|&s| (s, 0)).collect();
        let mut results = Vec::new();

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for edge in self.graph.edges_directed(node, direction) {
                if !matches!(edge.weight(), EdgeWeight::Calls) {
                    continue;
                }
                let neighbor = match direction {
                    Direction::Outgoing => edge.target(),
                    Direction::Incoming => edge.source(),
                };
                if visited.insert(neighbor) {
                    results.push(neighbor);
                    queue.push_back((neighbor, depth + 1));
                }
            }
        }

        results
    }

    /// Find a symbol node by file path and start line.
    pub fn find_symbol(&self, file_path: &str, start_line: u32) -> Option<NodeIndex> {
        self.symbol_nodes
            .get(&(file_path.to_string(), start_line))
            .copied()
    }

    /// Find all symbol nodes with a given name.
    pub fn find_symbols_by_name(&self, name: &str) -> &[NodeIndex] {
        self.symbols_by_name
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

}
