pub mod builder;
pub mod cfg;
pub mod cfg_languages;
pub mod resource;
pub mod taint;

use std::collections::{HashMap, HashSet};

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::audit::project_index::{ExportedSymbol, FileEntry, GraphNode, ProjectIndex};
use crate::language::Language;
use crate::models::SymbolKind;

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

    // --- Compat methods for ProjectIndex migration (Phase 2) ---

    /// Build a file dependency edge map (same shape as old `ProjectIndex.edges`).
    pub fn file_dependency_edges(&self) -> HashMap<GraphNode, HashSet<GraphNode>> {
        let mut edges: HashMap<GraphNode, HashSet<GraphNode>> = HashMap::new();
        for edge_idx in self.graph.edge_indices() {
            if let Some(EdgeWeight::Imports) = self.graph.edge_weight(edge_idx) {
                let (from, to) = self.graph.edge_endpoints(edge_idx).unwrap();
                if let (Some(from_path), Some(to_path)) =
                    (self.file_node_path(from), self.file_node_path(to))
                {
                    edges
                        .entry(GraphNode::File(from_path))
                        .or_default()
                        .insert(GraphNode::File(to_path));
                }
            }
        }
        edges
    }

    /// Build reverse file dependency edges.
    pub fn reverse_file_edges(&self) -> HashMap<GraphNode, HashSet<GraphNode>> {
        let mut reverse: HashMap<GraphNode, HashSet<GraphNode>> = HashMap::new();
        for edge_idx in self.graph.edge_indices() {
            if let Some(EdgeWeight::Imports) = self.graph.edge_weight(edge_idx) {
                let (from, to) = self.graph.edge_endpoints(edge_idx).unwrap();
                if let (Some(from_path), Some(to_path)) =
                    (self.file_node_path(from), self.file_node_path(to))
                {
                    reverse
                        .entry(GraphNode::File(to_path))
                        .or_default()
                        .insert(GraphNode::File(from_path));
                }
            }
        }
        reverse
    }

    /// Build a map of file entries (same shape as old `ProjectIndex.files`).
    pub fn file_entries(&self) -> HashMap<String, FileEntry> {
        let mut entries = HashMap::new();
        for (&ref path, &file_idx) in &self.file_nodes {
            let language = match &self.graph[file_idx] {
                NodeWeight::File { language, .. } => *language,
                _ => continue,
            };

            // Collect symbols for this file
            let mut symbol_count = 0;
            let mut exported_symbols = Vec::new();
            let mut line_count = 0u32;

            for edge in self.graph.edges_directed(file_idx, Direction::Outgoing) {
                let target = edge.target();
                match &self.graph[target] {
                    NodeWeight::Symbol {
                        name,
                        kind,
                        start_line,
                        end_line,
                        exported,
                        ..
                    } => {
                        symbol_count += 1;
                        if *end_line > line_count {
                            line_count = *end_line;
                        }
                        if *exported {
                            exported_symbols.push(ExportedSymbol {
                                name: name.clone(),
                                kind: *kind,
                                signature: None,
                                start_line: *start_line,
                            });
                        }
                    }
                    _ => {}
                }
            }

            entries.insert(
                path.clone(),
                FileEntry {
                    path: path.clone(),
                    language,
                    line_count,
                    symbol_count,
                    exported_symbols,
                    imports: Vec::new(),
                },
            );
        }
        entries
    }

    /// Convert to a full ProjectIndex (for backward compat with analyzers).
    pub fn to_project_index(&self) -> ProjectIndex {
        let mut index = ProjectIndex::new();
        index.files = self.file_entries();
        index.edges = self.file_dependency_edges();
        index.known_files = self.file_nodes.keys().cloned().collect();
        index
    }

    fn file_node_path(&self, idx: NodeIndex) -> Option<String> {
        match &self.graph[idx] {
            NodeWeight::File { path, .. } => Some(path.clone()),
            _ => None,
        }
    }
}
