pub mod builder;
pub mod cfg;
pub mod intern;
pub mod metrics;
pub mod resource;
pub mod taint;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

pub use intern::{Spur, Symbols};

use crate::language::Language;
use crate::models::SymbolKind;
use crate::storage::workspace::Workspace;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfgExitKind {
    Normal,
    TrueBranch,
    FalseBranch,
    Exception,
    Cleanup,
}

impl CfgExitKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::TrueBranch => "true_branch",
            Self::FalseBranch => "false_branch",
            Self::Exception => "exception",
            Self::Cleanup => "cleanup",
        }
    }

    pub fn from_cfg_edge(e: &cfg::CfgEdge) -> Self {
        match e {
            cfg::CfgEdge::Normal => Self::Normal,
            cfg::CfgEdge::TrueBranch => Self::TrueBranch,
            cfg::CfgEdge::FalseBranch => Self::FalseBranch,
            cfg::CfgEdge::Exception => Self::Exception,
            cfg::CfgEdge::Cleanup => Self::Cleanup,
        }
    }
}

#[derive(Debug, Clone)]
pub enum NodeWeight {
    File {
        path: Spur,
        language: Language,
    },
    Symbol {
        name: Spur,
        kind: SymbolKind,
        file_path: Spur,
        start_line: u32,
        end_line: u32,
        exported: bool,
    },
    CallSite {
        name: Spur,
        file_path: Spur,
        line: u32,
        /// Literal arguments at this call site (strings/numbers/bools only).
        arg_literals: Vec<Spur>,
        /// Name of the enclosing test function, when this call site sits
        /// inside a test (path matches `is_test_file` and the enclosing
        /// symbol's name follows a test naming convention).
        enclosing_test_name: Option<Spur>,
        /// The Symbol node that contains this call site, if any.
        caller_symbol: Option<NodeIndex>,
    },
    Parameter {
        name: Spur,
        function_node: NodeIndex,
        position: usize,
        is_taint_source: bool,
    },
    ExternalSource {
        kind: SourceKind,
        file_path: Spur,
        line: u32,
    },
    CfgExit {
        function_node: NodeIndex,
        function_name: Spur,
        file_path: Spur,
        line: u32,
        exit_kind: CfgExitKind,
        exit_label: Option<Spur>,
    },
}

#[derive(Debug, Clone)]
pub enum EdgeWeight {
    DefinedIn,
    Calls,
    Imports,
    FlowsTo,
    SanitizedBy { sanitizer: Spur },
    Exports,
    Acquires { resource_type: Spur },
    ReleasedBy,
    Contains,
    ExitsVia(CfgExitKind),
}

pub struct CodeGraph {
    pub graph: DiGraph<NodeWeight, EdgeWeight>,
    /// Shared string interner. Every interned `Spur` (file path, symbol name)
    /// is resolvable through this handle.
    pub symbols: Symbols,
    /// file_path (interned) -> NodeIndex
    pub file_nodes: HashMap<Spur, NodeIndex>,
    /// (file_path, start_line) -> NodeIndex
    pub symbol_nodes: HashMap<(Spur, u32), NodeIndex>,
    /// symbol name (interned) -> list of NodeIndex for all symbols with that name
    pub symbols_by_name: HashMap<Spur, Vec<NodeIndex>>,
    /// Set of function NodeIndex values for which a CFG can be built. The CFGs
    /// themselves are NOT stored — they are rebuilt on demand by
    /// `cfg_for_function` to keep boot-time memory low.
    pub function_cfg_indices: HashSet<NodeIndex>,
    /// Lazy CFG cache. Populated on first call to `cfg_for_function` for a
    /// given function. Tests may pre-populate via `inject_cfg`.
    cfg_cache: Mutex<HashMap<NodeIndex, cfg::FunctionCfg>>,
    /// Sentinel for the resource-lifecycle pass. `ensure_resource_graph` runs
    /// `ResourceAnalyzer::analyze_all` at most once.
    resource_analyzed: bool,
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
            symbols: Symbols::new(),
            file_nodes: HashMap::new(),
            symbol_nodes: HashMap::new(),
            symbols_by_name: HashMap::new(),
            function_cfg_indices: HashSet::new(),
            cfg_cache: Mutex::new(HashMap::new()),
            resource_analyzed: false,
        }
    }

    /// Fetch or rebuild the CFG for a function node.
    /// - Returns a cached CFG if one was previously built or injected.
    /// - Otherwise, if `workspace` is provided, re-parses the function's source
    ///   and constructs a fresh CFG, caches it, and returns it.
    /// - Returns `None` if the node is not a function, the workspace cannot
    ///   read its source, or the language has no CFG builder.
    pub fn cfg_for_function(
        &self,
        workspace: Option<&Workspace>,
        idx: NodeIndex,
    ) -> Option<cfg::FunctionCfg> {
        if let Ok(cache) = self.cfg_cache.lock()
            && let Some(c) = cache.get(&idx)
        {
            return Some(c.clone());
        }
        let workspace = workspace?;
        let (file_path, start_line, end_line) = match self.graph.node_weight(idx)? {
            NodeWeight::Symbol {
                file_path,
                start_line,
                end_line,
                ..
            } => (
                self.symbols.resolve(*file_path).to_string(),
                *start_line,
                *end_line,
            ),
            _ => return None,
        };
        let lang = workspace.file_language(&file_path)?;
        let builder = crate::languages::cfg::cfg_builder_for_language(lang)?;
        let source = workspace.read_file(&file_path)?;
        let mut parser = crate::parser::create_parser(lang).ok()?;
        let tree = parser.parse(&*source, None)?;
        let func_node = builder::find_node_at_line(tree.root_node(), start_line, end_line)?;
        let cfg = builder.build_cfg(&func_node, source.as_bytes()).ok()?;
        if let Ok(mut cache) = self.cfg_cache.lock() {
            cache.insert(idx, cfg.clone());
        }
        Some(cfg)
    }

    /// Inject a CFG into the lazy cache. Used by tests that synthesise CFGs
    /// without going through the builder, and by passes that want to share
    /// already-built CFGs.
    pub fn inject_cfg(&mut self, idx: NodeIndex, cfg: cfg::FunctionCfg) {
        self.function_cfg_indices.insert(idx);
        if let Ok(mut cache) = self.cfg_cache.lock() {
            cache.insert(idx, cfg);
        }
    }

    /// Run resource-lifecycle analysis (`Acquires` / `ReleasedBy` edges)
    /// if it hasn't been run yet. Idempotent: subsequent calls are no-ops.
    pub fn ensure_resource_graph(&mut self, workspace: Option<&Workspace>) {
        if self.resource_analyzed {
            return;
        }
        resource::ResourceAnalyzer::analyze_all(self, workspace);
        self.resource_analyzed = true;
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
