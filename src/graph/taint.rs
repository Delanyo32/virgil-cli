use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use super::cfg::{CfgStatementKind, FunctionCfg};
use super::{CodeGraph, EdgeWeight, NodeWeight, SourceKind};

// ---------------------------------------------------------------------------
// Source / sink / sanitizer tables
// ---------------------------------------------------------------------------

/// Taint source patterns — user inputs, env vars, network reads, etc.
/// Each entry is matched via `contains` against variable names, call targets,
/// or parameter names encountered in the CFG.
const SOURCES: &[TaintPattern] = &[
    // -- JavaScript / TypeScript --
    TaintPattern { text: "request.body",   kind: SourceKind::UserInput },
    TaintPattern { text: "request.params", kind: SourceKind::UserInput },
    TaintPattern { text: "request.query",  kind: SourceKind::UserInput },
    TaintPattern { text: "request.args",   kind: SourceKind::UserInput },
    TaintPattern { text: "req.body",       kind: SourceKind::UserInput },
    TaintPattern { text: "req.params",     kind: SourceKind::UserInput },
    TaintPattern { text: "req.query",      kind: SourceKind::UserInput },
    TaintPattern { text: "req.headers",    kind: SourceKind::UserInput },
    TaintPattern { text: "process.env",    kind: SourceKind::EnvironmentVar },
    TaintPattern { text: "document.cookie",kind: SourceKind::UserInput },
    TaintPattern { text: "location.href",  kind: SourceKind::UserInput },
    TaintPattern { text: "location.search",kind: SourceKind::UserInput },
    TaintPattern { text: "window.location",kind: SourceKind::UserInput },
    // -- Python --
    TaintPattern { text: "request.form",   kind: SourceKind::UserInput },
    TaintPattern { text: "request.data",   kind: SourceKind::UserInput },
    TaintPattern { text: "request.json",   kind: SourceKind::UserInput },
    TaintPattern { text: "request.args",   kind: SourceKind::UserInput },
    TaintPattern { text: "os.environ",     kind: SourceKind::EnvironmentVar },
    TaintPattern { text: "sys.argv",       kind: SourceKind::UserInput },
    TaintPattern { text: "sys.stdin",      kind: SourceKind::UserInput },
    TaintPattern { text: "input(",         kind: SourceKind::UserInput },
    // -- Rust --
    TaintPattern { text: "env::var",       kind: SourceKind::EnvironmentVar },
    TaintPattern { text: "env::args",      kind: SourceKind::UserInput },
    TaintPattern { text: "stdin",          kind: SourceKind::UserInput },
    // -- Go --
    TaintPattern { text: "os.Getenv",      kind: SourceKind::EnvironmentVar },
    TaintPattern { text: "os.Args",        kind: SourceKind::UserInput },
    TaintPattern { text: "r.FormValue",    kind: SourceKind::UserInput },
    TaintPattern { text: "r.URL.Query",    kind: SourceKind::UserInput },
    TaintPattern { text: "r.Header",       kind: SourceKind::UserInput },
    TaintPattern { text: "r.Body",         kind: SourceKind::UserInput },
    TaintPattern { text: "os.Stdin",       kind: SourceKind::UserInput },
    // -- PHP --
    TaintPattern { text: "$_GET",          kind: SourceKind::UserInput },
    TaintPattern { text: "$_POST",         kind: SourceKind::UserInput },
    TaintPattern { text: "$_REQUEST",      kind: SourceKind::UserInput },
    TaintPattern { text: "$_COOKIE",       kind: SourceKind::UserInput },
    TaintPattern { text: "$_SERVER",       kind: SourceKind::UserInput },
    TaintPattern { text: "$_FILES",        kind: SourceKind::UserInput },
    TaintPattern { text: "getenv",         kind: SourceKind::EnvironmentVar },
    TaintPattern { text: "php://input",    kind: SourceKind::UserInput },
    // -- C / C++ --
    TaintPattern { text: "getenv",         kind: SourceKind::EnvironmentVar },
    TaintPattern { text: "argv",           kind: SourceKind::UserInput },
    TaintPattern { text: "fgets",          kind: SourceKind::UserInput },
    TaintPattern { text: "scanf",          kind: SourceKind::UserInput },
    TaintPattern { text: "gets",           kind: SourceKind::UserInput },
    TaintPattern { text: "fread",          kind: SourceKind::FileRead },
    TaintPattern { text: "recv",           kind: SourceKind::NetworkRead },
    TaintPattern { text: "recvfrom",       kind: SourceKind::NetworkRead },
    // -- Java --
    TaintPattern { text: "getParameter",   kind: SourceKind::UserInput },
    TaintPattern { text: "getHeader",      kind: SourceKind::UserInput },
    TaintPattern { text: "getInputStream", kind: SourceKind::UserInput },
    TaintPattern { text: "getReader",      kind: SourceKind::UserInput },
    TaintPattern { text: "System.getenv",  kind: SourceKind::EnvironmentVar },
    TaintPattern { text: "System.getProperty", kind: SourceKind::EnvironmentVar },
    // -- C# --
    TaintPattern { text: "Request.Query",  kind: SourceKind::UserInput },
    TaintPattern { text: "Request.Form",   kind: SourceKind::UserInput },
    TaintPattern { text: "Request.Headers",kind: SourceKind::UserInput },
    TaintPattern { text: "Request.Body",   kind: SourceKind::UserInput },
    TaintPattern { text: "Environment.GetEnvironmentVariable", kind: SourceKind::EnvironmentVar },
    // -- Deserialization --
    TaintPattern { text: "JSON.parse",     kind: SourceKind::Deserialization },
    TaintPattern { text: "json.loads",     kind: SourceKind::Deserialization },
    TaintPattern { text: "json_decode",    kind: SourceKind::Deserialization },
    TaintPattern { text: "serde_json::from", kind: SourceKind::Deserialization },
    TaintPattern { text: "ObjectMapper.readValue", kind: SourceKind::Deserialization },
    // -- Database reads --
    TaintPattern { text: "ResultSet.getString", kind: SourceKind::DatabaseRead },
    TaintPattern { text: "fetchone",       kind: SourceKind::DatabaseRead },
    TaintPattern { text: "fetchall",       kind: SourceKind::DatabaseRead },
    TaintPattern { text: "rows.Scan",      kind: SourceKind::DatabaseRead },
];

/// Sink patterns — dangerous operations that should not receive unsanitized input.
const SINKS: &[&str] = &[
    // SQL injection
    "execute", "query", "raw_query", "cursor.execute", "db.query",
    "prepare", "exec", "raw",
    // Command injection
    "system", "exec.Command", "subprocess.call", "subprocess.run",
    "subprocess.Popen", "Runtime.exec", "shell_exec", "popen",
    "execvp", "execve", "ProcessBuilder",
    "os.system", "os.popen", "os.exec",
    "child_process.exec", "child_process.spawn",
    "Command::new",
    // Code injection
    "eval", "Function(", "compile",
    // XSS / DOM
    "innerHTML", "outerHTML", "document.write", "dangerouslySetInnerHTML",
    // Path traversal
    "readFile", "writeFile", "open", "readFileSync", "writeFileSync",
    // PHP specific
    "mysqli_query", "pg_query", "preg_replace",
    "include", "require", "include_once", "require_once",
    // LDAP
    "ldap_search", "ldap_bind",
    // Deserialization sinks
    "unserialize", "pickle.loads", "yaml.load",
    "ObjectInputStream",
    // Network sinks
    "fetch", "XMLHttpRequest", "http.Get", "http.Post",
    "redirect", "header(",
];

/// Sanitizer patterns — functions that neutralize taint.
const SANITIZERS: &[&str] = &[
    // Generic
    "escape", "sanitize", "validate", "encode", "clean",
    "purify", "strip", "filter",
    // JS / TS
    "parseInt", "parseFloat", "Number(", "encodeURIComponent", "encodeURI",
    "DOMPurify.sanitize", "xss(",
    // Python
    "bleach.clean", "markupsafe.escape", "shlex.quote", "quote",
    // Go
    "filepath.Clean", "html.EscapeString", "url.QueryEscape",
    "template.HTMLEscapeString", "strconv.Atoi", "strconv.ParseInt",
    // PHP
    "htmlspecialchars", "htmlentities", "addslashes",
    "mysqli_real_escape_string", "pg_escape_string",
    "filter_var", "filter_input",
    // Java
    "PreparedStatement", "parameterize",
    "StringEscapeUtils.escapeHtml", "StringEscapeUtils.escapeSql",
    "Encoder.encodeForHTML", "ESAPI.encoder",
    // C / C++
    "snprintf", "strlcpy",
    // Rust
    "html_escape", "ammonia::clean",
    // C#
    "HtmlEncode", "UrlEncode", "AntiXss",
    "SqlParameter", "AddWithValue",
    // General ORM / prepared statements
    "parameterized", "bind_param", "bindValue", "bindParam",
    "placeholder", "prepare(",
];

/// A taint source pattern with its classification.
struct TaintPattern {
    text: &'static str,
    kind: SourceKind,
}

// ---------------------------------------------------------------------------
// Taint finding — output of the analysis
// ---------------------------------------------------------------------------

/// A single taint finding: unsanitized data flowing from source to sink.
#[derive(Debug, Clone)]
pub struct TaintFinding {
    /// The function graph node where the finding was detected.
    pub function_node: NodeIndex,
    /// Human-readable name of the function.
    pub function_name: String,
    /// File path containing the function.
    pub file_path: String,
    /// The variable that carried taint into the sink.
    pub tainted_var: String,
    /// The sink call name.
    pub sink_name: String,
    /// Line of the sink call.
    pub sink_line: u32,
    /// How the variable became tainted (source description).
    pub source_description: String,
    /// Line where taint originated (if known).
    pub source_line: Option<u32>,
}

// ---------------------------------------------------------------------------
// Taint state — per-variable tracking during analysis
// ---------------------------------------------------------------------------

/// Provenance of a tainted value.
#[derive(Debug, Clone)]
struct TaintOrigin {
    description: String,
    line: Option<u32>,
}

/// Taint state for a single program point.
#[derive(Debug, Clone, Default)]
struct TaintState {
    /// variable name -> how it became tainted
    tainted: HashMap<String, TaintOrigin>,
}

impl TaintState {
    fn is_tainted(&self, var: &str) -> bool {
        self.tainted.contains_key(var)
    }

    fn mark_tainted(&mut self, var: &str, origin: TaintOrigin) {
        self.tainted.insert(var.to_string(), origin);
    }

    fn remove_taint(&mut self, var: &str) {
        self.tainted.remove(var);
    }

    /// Merge another state into this one (union semantics).
    fn merge(&mut self, other: &TaintState) {
        for (var, origin) in &other.tainted {
            // If already tainted, keep existing origin (first wins).
            self.tainted
                .entry(var.clone())
                .or_insert_with(|| origin.clone());
        }
    }

    fn any_tainted<'a>(&'a self, vars: &'a [String]) -> Option<(&'a str, &'a TaintOrigin)> {
        for var in vars {
            if let Some(origin) = self.tainted.get(var.as_str()) {
                return Some((var.as_str(), origin));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// TaintEngine
// ---------------------------------------------------------------------------

pub struct TaintEngine;

impl TaintEngine {
    /// Run taint analysis on all functions that have CFGs in the graph.
    /// Adds FlowsTo and SanitizedBy edges, and returns all findings.
    pub fn analyze_all(graph: &mut CodeGraph) -> Vec<TaintFinding> {
        // Collect the function nodes to analyze (we need to avoid borrowing
        // graph mutably while iterating).
        let func_nodes: Vec<(NodeIndex, String, String)> = graph
            .function_cfgs
            .keys()
            .filter_map(|&node_idx| {
                match &graph.graph[node_idx] {
                    NodeWeight::Symbol {
                        name, file_path, ..
                    } => Some((node_idx, name.clone(), file_path.clone())),
                    _ => None,
                }
            })
            .collect();

        let mut all_findings = Vec::new();

        for (func_idx, func_name, file_path) in &func_nodes {
            // We need to clone the CFG to avoid borrow conflicts.
            let cfg = match graph.function_cfgs.get(func_idx) {
                Some(cfg) => cfg.clone(),
                None => continue,
            };

            // Collect parameter nodes for this function.
            let param_names = collect_parameter_names(graph, *func_idx);

            let (findings, edges) =
                Self::analyze_function(*func_idx, func_name, file_path, &cfg, &param_names);

            all_findings.extend(findings);

            // Apply edges to the graph.
            for edge_action in edges {
                match edge_action {
                    GraphEdgeAction::FlowsTo { from, to } => {
                        graph.graph.add_edge(from, to, EdgeWeight::FlowsTo);
                    }
                    GraphEdgeAction::SanitizedBy {
                        node,
                        target,
                        sanitizer,
                    } => {
                        graph.graph.add_edge(
                            node,
                            target,
                            EdgeWeight::SanitizedBy { sanitizer },
                        );
                    }
                    GraphEdgeAction::AddExternalSource {
                        kind,
                        file_path,
                        line,
                        target_node,
                    } => {
                        let src_idx = graph.graph.add_node(NodeWeight::ExternalSource {
                            kind,
                            file_path,
                            line,
                        });
                        graph.graph.add_edge(src_idx, target_node, EdgeWeight::FlowsTo);
                    }
                }
            }
        }

        // Mark parameter nodes as taint sources where applicable.
        mark_parameter_taint_sources(graph);

        all_findings
    }

    /// Analyze a single function's CFG for taint propagation.
    fn analyze_function(
        func_idx: NodeIndex,
        func_name: &str,
        file_path: &str,
        cfg: &FunctionCfg,
        param_names: &[String],
    ) -> (Vec<TaintFinding>, Vec<GraphEdgeAction>) {
        let mut findings = Vec::new();
        let mut edges = Vec::new();

        // Compute a topological order of the CFG blocks. If the CFG has cycles
        // (loops), fall back to BFS from entry with a fixed-point iteration.
        let block_order = topo_order_or_bfs(cfg);

        // Per-block input taint state, keyed by block NodeIndex.
        let mut block_states: HashMap<NodeIndex, TaintState> = HashMap::new();

        // Initialize the entry block state with tainted parameters.
        let mut entry_state = TaintState::default();
        for param in param_names {
            if is_source_param(param) {
                entry_state.mark_tainted(
                    param,
                    TaintOrigin {
                        description: format!("parameter '{param}' matches taint source pattern"),
                        line: None,
                    },
                );
            }
        }
        block_states.insert(cfg.entry, entry_state);

        // Fixed-point iteration (handles loops). Most acyclic CFGs converge
        // in a single pass.
        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 20;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            // Clear findings and edges each iteration — only the final
            // (converged) pass's results are kept.  Without this, earlier
            // iterations would duplicate sink findings.
            findings.clear();
            edges.clear();

            for &block_idx in &block_order {
                // Merge incoming states from predecessors.
                let mut state = TaintState::default();
                for edge in cfg.blocks.edges_directed(block_idx, Direction::Incoming) {
                    if let Some(pred_state) = block_states.get(&edge.source()) {
                        state.merge(pred_state);
                    }
                }
                // Also merge the existing state for this block (for entry block init).
                if let Some(existing) = block_states.get(&block_idx) {
                    state.merge(existing);
                }

                let block = &cfg.blocks[block_idx];

                // Process each statement in the block.
                for stmt in &block.statements {
                    match &stmt.kind {
                        CfgStatementKind::Assignment { target, source_vars } => {
                            // Check if any source variable is tainted.
                            if let Some((tainted_var, origin)) = state.any_tainted(source_vars) {
                                state.mark_tainted(
                                    target,
                                    TaintOrigin {
                                        description: format!(
                                            "assigned from tainted '{tainted_var}' ({})",
                                            origin.description
                                        ),
                                        line: Some(stmt.line),
                                    },
                                );
                            }
                            // Check if any source_var is itself a source expression.
                            for sv in source_vars {
                                if let Some(src_kind) = matches_source(sv) {
                                    state.mark_tainted(
                                        target,
                                        TaintOrigin {
                                            description: format!(
                                                "assigned from taint source '{sv}'"
                                            ),
                                            line: Some(stmt.line),
                                        },
                                    );
                                    edges.push(GraphEdgeAction::AddExternalSource {
                                        kind: src_kind,
                                        file_path: file_path.to_string(),
                                        line: stmt.line,
                                        target_node: func_idx,
                                    });
                                }
                            }
                        }

                        CfgStatementKind::Call { name, args } => {
                            // 1) Check if this call is a source.
                            if let Some(src_kind) = matches_source(name) {
                                // If the call result is captured, it would show up as
                                // an Assignment. Mark all args as potentially tainted
                                // for downstream propagation.
                                for arg in args {
                                    state.mark_tainted(
                                        arg,
                                        TaintOrigin {
                                            description: format!(
                                                "return value of taint source '{name}'"
                                            ),
                                            line: Some(stmt.line),
                                        },
                                    );
                                }
                                edges.push(GraphEdgeAction::AddExternalSource {
                                    kind: src_kind,
                                    file_path: file_path.to_string(),
                                    line: stmt.line,
                                    target_node: func_idx,
                                });
                            }

                            // 2) Check if this call is a sanitizer.
                            if matches_sanitizer(name) {
                                for arg in args {
                                    if state.is_tainted(arg) {
                                        state.remove_taint(arg);
                                        edges.push(GraphEdgeAction::SanitizedBy {
                                            node: func_idx,
                                            target: func_idx,
                                            sanitizer: name.clone(),
                                        });
                                    }
                                }
                            }

                            // 3) Check if this call is a sink with tainted args.
                            if matches_sink(name) {
                                if let Some((tainted_var, origin)) = state.any_tainted(args) {
                                    findings.push(TaintFinding {
                                        function_node: func_idx,
                                        function_name: func_name.to_string(),
                                        file_path: file_path.to_string(),
                                        tainted_var: tainted_var.to_string(),
                                        sink_name: name.clone(),
                                        sink_line: stmt.line,
                                        source_description: origin.description.clone(),
                                        source_line: origin.line,
                                    });

                                    // Add a FlowsTo edge from function to itself
                                    // to mark that taint reaches a sink here.
                                    edges.push(GraphEdgeAction::FlowsTo {
                                        from: func_idx,
                                        to: func_idx,
                                    });
                                }
                            }

                            // 4) Taint propagation through unknown calls:
                            //    If any arg is tainted, conservatively assume the
                            //    call may return tainted data (captured elsewhere
                            //    via Assignment). We don't change state here since
                            //    the result capture is handled by Assignment.
                        }

                        CfgStatementKind::Return { value_vars } => {
                            // If returning tainted data, note it (useful for
                            // future inter-procedural analysis).
                            if state.any_tainted(value_vars).is_some() {
                                edges.push(GraphEdgeAction::FlowsTo {
                                    from: func_idx,
                                    to: func_idx,
                                });
                            }
                        }

                        CfgStatementKind::Guard { condition_vars: _ } => {
                            // Guards don't propagate or sanitize taint.
                        }

                        CfgStatementKind::ResourceAcquire { target, resource_type } => {
                            // If the resource type looks like a source, taint
                            // the target variable.
                            if let Some(src_kind) = matches_source(resource_type) {
                                state.mark_tainted(
                                    target,
                                    TaintOrigin {
                                        description: format!(
                                            "acquired resource '{resource_type}' is a taint source"
                                        ),
                                        line: Some(stmt.line),
                                    },
                                );
                                edges.push(GraphEdgeAction::AddExternalSource {
                                    kind: src_kind,
                                    file_path: file_path.to_string(),
                                    line: stmt.line,
                                    target_node: func_idx,
                                });
                            }
                        }

                        CfgStatementKind::ResourceRelease { .. } => {
                            // Releases don't affect taint.
                        }

                        CfgStatementKind::PhiNode { target, sources } => {
                            // Phi merges: if any source is tainted, target is tainted.
                            if let Some((tainted_var, origin)) = state.any_tainted(sources) {
                                state.mark_tainted(
                                    target,
                                    TaintOrigin {
                                        description: format!(
                                            "phi merge from tainted '{tainted_var}' ({})",
                                            origin.description
                                        ),
                                        line: Some(stmt.line),
                                    },
                                );
                            }
                        }
                    }
                }

                // Update the block's output state. If it changed, mark for
                // another iteration.
                let prev_count = block_states
                    .get(&block_idx)
                    .map_or(0, |s| s.tainted.len());
                let new_count = state.tainted.len();

                // Check if any new taint was added (monotonic growth).
                let prev_keys: HashSet<&String> = block_states
                    .get(&block_idx)
                    .map(|s| s.tainted.keys().collect())
                    .unwrap_or_default();
                let new_keys: HashSet<&String> = state.tainted.keys().collect();

                if new_keys != prev_keys || new_count != prev_count {
                    changed = true;
                }

                block_states.insert(block_idx, state);
            }
        }

        (findings, edges)
    }
}

// ---------------------------------------------------------------------------
// Graph edge actions — deferred mutations to avoid borrow conflicts
// ---------------------------------------------------------------------------

enum GraphEdgeAction {
    FlowsTo {
        from: NodeIndex,
        to: NodeIndex,
    },
    SanitizedBy {
        node: NodeIndex,
        target: NodeIndex,
        sanitizer: String,
    },
    AddExternalSource {
        kind: SourceKind,
        file_path: String,
        line: u32,
        target_node: NodeIndex,
    },
}

// ---------------------------------------------------------------------------
// Helper: collect parameter names for a function node
// ---------------------------------------------------------------------------

fn collect_parameter_names(graph: &CodeGraph, func_idx: NodeIndex) -> Vec<String> {
    let mut params = Vec::new();
    for edge in graph.graph.edges_directed(func_idx, Direction::Incoming) {
        if let NodeWeight::Parameter { name, .. } = &graph.graph[edge.source()] {
            params.push(name.clone());
        }
    }
    // Also check outgoing edges (some builders may use Contains direction).
    for edge in graph.graph.edges_directed(func_idx, Direction::Outgoing) {
        if let NodeWeight::Parameter { name, .. } = &graph.graph[edge.target()] {
            params.push(name.clone());
        }
    }
    params
}

/// After analysis, mark Parameter nodes as taint sources based on the tables.
fn mark_parameter_taint_sources(graph: &mut CodeGraph) {
    let param_indices: Vec<NodeIndex> = graph
        .graph
        .node_indices()
        .filter(|&idx| matches!(graph.graph[idx], NodeWeight::Parameter { .. }))
        .collect();

    for idx in param_indices {
        if let NodeWeight::Parameter {
            ref name,
            ref mut is_taint_source,
            ..
        } = graph.graph[idx]
        {
            *is_taint_source = is_source_param(name);
        }
    }
}

// ---------------------------------------------------------------------------
// Pattern matching helpers
// ---------------------------------------------------------------------------

/// Check if a string matches any taint source pattern. Returns the SourceKind.
fn matches_source(text: &str) -> Option<SourceKind> {
    let lower = text.to_lowercase();
    for pat in SOURCES {
        if lower.contains(&pat.text.to_lowercase()) {
            return Some(pat.kind.clone());
        }
    }
    None
}

/// Check if a call name matches any sink pattern.
fn matches_sink(name: &str) -> bool {
    let lower = name.to_lowercase();
    for &sink in SINKS {
        let sink_lower = sink.to_lowercase();
        // Match if the call name contains the sink pattern, or if the
        // last segment (after `.` or `::`) matches.
        if lower.contains(&sink_lower) {
            return true;
        }
        // Also check the final segment of the call name.
        let final_segment = name
            .rsplit_once('.')
            .or_else(|| name.rsplit_once("::"))
            .map(|(_, s)| s)
            .unwrap_or(name);
        if final_segment.to_lowercase() == sink_lower.trim_end_matches('(') {
            return true;
        }
    }
    false
}

/// Check if a call name matches any sanitizer pattern.
fn matches_sanitizer(name: &str) -> bool {
    let lower = name.to_lowercase();
    for &san in SANITIZERS {
        let san_lower = san.to_lowercase();
        if lower.contains(&san_lower) {
            return true;
        }
        let final_segment = name
            .rsplit_once('.')
            .or_else(|| name.rsplit_once("::"))
            .map(|(_, s)| s)
            .unwrap_or(name);
        if final_segment.to_lowercase() == san_lower.trim_end_matches('(') {
            return true;
        }
    }
    false
}

/// Check if a parameter name suggests it carries user-controlled data.
fn is_source_param(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Common parameter names that typically carry user input.
    const PARAM_PATTERNS: &[&str] = &[
        "request", "req", "input", "body", "query", "params",
        "args", "argv", "data", "payload", "form", "user_input",
        "raw_input", "stdin",
    ];
    for &pat in PARAM_PATTERNS {
        if lower == pat || lower.starts_with(&format!("{pat}_")) || lower.ends_with(&format!("_{pat}")) {
            return true;
        }
    }
    // Also check against the full source table (for compound names like
    // "request_body").
    matches_source(name).is_some()
}

// ---------------------------------------------------------------------------
// Topological ordering with cycle handling
// ---------------------------------------------------------------------------

/// Compute a processing order for CFG blocks. Uses Kahn's algorithm for
/// topological sort. If cycles exist (loops), falls back to a BFS order
/// from the entry node, which still visits every reachable block.
fn topo_order_or_bfs(cfg: &FunctionCfg) -> Vec<NodeIndex> {
    let blocks = &cfg.blocks;
    let node_count = blocks.node_count();
    if node_count == 0 {
        return Vec::new();
    }

    // Kahn's algorithm
    let mut in_degree: HashMap<NodeIndex, usize> = HashMap::with_capacity(node_count);
    for idx in blocks.node_indices() {
        in_degree.insert(idx, 0);
    }
    for edge in blocks.edge_references() {
        *in_degree.entry(edge.target()).or_insert(0) += 1;
    }

    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    // Start with nodes that have zero in-degree, preferring the entry block.
    if in_degree.get(&cfg.entry).copied().unwrap_or(0) == 0 {
        queue.push_back(cfg.entry);
    }
    for (&idx, &deg) in &in_degree {
        if deg == 0 && idx != cfg.entry {
            queue.push_back(idx);
        }
    }

    let mut order = Vec::with_capacity(node_count);
    let mut visited: HashSet<NodeIndex> = HashSet::with_capacity(node_count);

    while let Some(node) = queue.pop_front() {
        if !visited.insert(node) {
            continue;
        }
        order.push(node);
        for edge in blocks.edges_directed(node, Direction::Outgoing) {
            let target = edge.target();
            if let Some(deg) = in_degree.get_mut(&target) {
                *deg = deg.saturating_sub(1);
                if *deg == 0 && !visited.contains(&target) {
                    queue.push_back(target);
                }
            }
        }
    }

    // If some blocks were not visited (part of a cycle), add them via BFS
    // from the entry.
    if order.len() < node_count {
        let mut bfs_queue = VecDeque::new();
        bfs_queue.push_back(cfg.entry);
        let mut bfs_visited: HashSet<NodeIndex> = HashSet::new();
        let mut bfs_order = Vec::new();

        while let Some(node) = bfs_queue.pop_front() {
            if !bfs_visited.insert(node) {
                continue;
            }
            bfs_order.push(node);
            for edge in blocks.edges_directed(node, Direction::Outgoing) {
                let target = edge.target();
                if !bfs_visited.contains(&target) {
                    bfs_queue.push_back(target);
                }
            }
        }

        // Add any remaining unreachable nodes (shouldn't happen in practice).
        for idx in blocks.node_indices() {
            if !bfs_visited.contains(&idx) {
                bfs_order.push(idx);
            }
        }

        return bfs_order;
    }

    order
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};
    use crate::graph::{CodeGraph, NodeWeight};
    use crate::language::Language;
    use crate::models::SymbolKind;
    use petgraph::graph::DiGraph;

    /// Helper to build a minimal CodeGraph with one function and its CFG.
    fn make_graph_with_cfg(
        func_name: &str,
        stmts: Vec<CfgStatement>,
    ) -> (CodeGraph, NodeIndex) {
        let mut graph = CodeGraph::new();

        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "test.py".to_string(),
            language: Language::Python,
        });
        graph
            .file_nodes
            .insert("test.py".to_string(), file_idx);

        let func_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: func_name.to_string(),
            kind: SymbolKind::Function,
            file_path: "test.py".to_string(),
            start_line: 1,
            end_line: 10,
            exported: true,
        });
        graph
            .symbol_nodes
            .insert(("test.py".to_string(), 1), func_idx);
        graph
            .symbols_by_name
            .entry(func_name.to_string())
            .or_default()
            .push(func_idx);

        // Build a single-block CFG.
        let mut cfg_graph = DiGraph::new();
        let mut block = BasicBlock::new();
        block.statements = stmts;
        let entry = cfg_graph.add_node(block);

        let cfg = FunctionCfg {
            blocks: cfg_graph,
            entry,
            exits: vec![entry],
        };

        graph.function_cfgs.insert(func_idx, cfg);

        (graph, func_idx)
    }

    #[test]
    fn test_source_matching() {
        assert!(matches_source("request.body").is_some());
        assert!(matches_source("req.query").is_some());
        assert!(matches_source("os.environ").is_some());
        assert!(matches_source("env::var").is_some());
        assert!(matches_source("$_GET").is_some());
        assert!(matches_source("safe_value").is_none());
    }

    #[test]
    fn test_sink_matching() {
        assert!(matches_sink("execute"));
        assert!(matches_sink("cursor.execute"));
        assert!(matches_sink("db.query"));
        assert!(matches_sink("eval"));
        assert!(matches_sink("innerHTML"));
        assert!(!matches_sink("safeMethod"));
    }

    #[test]
    fn test_sanitizer_matching() {
        assert!(matches_sanitizer("escape"));
        assert!(matches_sanitizer("htmlspecialchars"));
        assert!(matches_sanitizer("parseInt"));
        assert!(matches_sanitizer("DOMPurify.sanitize"));
        assert!(!matches_sanitizer("processData"));
    }

    #[test]
    fn test_simple_taint_flow_to_sink() {
        // user_input = request.body  (tainted)
        // execute(user_input)        (sink with tainted arg)
        let stmts = vec![
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "user_input".to_string(),
                    source_vars: vec!["request.body".to_string()],
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "execute".to_string(),
                    args: vec!["user_input".to_string()],
                },
                line: 3,
            },
        ];

        let (mut graph, _func_idx) = make_graph_with_cfg("handle_request", stmts);
        let findings = TaintEngine::analyze_all(&mut graph);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].sink_name, "execute");
        assert_eq!(findings[0].tainted_var, "user_input");
        assert_eq!(findings[0].sink_line, 3);
    }

    #[test]
    fn test_sanitizer_removes_taint() {
        // user_input = request.body  (tainted)
        // escape(user_input)         (sanitizer removes taint)
        // execute(user_input)        (sink — but no longer tainted)
        let stmts = vec![
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "user_input".to_string(),
                    source_vars: vec!["request.body".to_string()],
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "escape".to_string(),
                    args: vec!["user_input".to_string()],
                },
                line: 3,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "execute".to_string(),
                    args: vec!["user_input".to_string()],
                },
                line: 4,
            },
        ];

        let (mut graph, _func_idx) = make_graph_with_cfg("handle_request", stmts);
        let findings = TaintEngine::analyze_all(&mut graph);

        assert!(findings.is_empty(), "expected no findings after sanitization");
    }

    #[test]
    fn test_taint_propagation_through_assignment() {
        // a = request.body       (tainted)
        // b = a                  (tainted via assignment)
        // c = b                  (tainted via assignment)
        // query(c)               (sink with tainted arg)
        let stmts = vec![
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "a".to_string(),
                    source_vars: vec!["request.body".to_string()],
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "b".to_string(),
                    source_vars: vec!["a".to_string()],
                },
                line: 3,
            },
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "c".to_string(),
                    source_vars: vec!["b".to_string()],
                },
                line: 4,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "query".to_string(),
                    args: vec!["c".to_string()],
                },
                line: 5,
            },
        ];

        let (mut graph, _func_idx) = make_graph_with_cfg("process", stmts);
        let findings = TaintEngine::analyze_all(&mut graph);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].tainted_var, "c");
        assert_eq!(findings[0].sink_name, "query");
    }

    #[test]
    fn test_no_finding_when_no_taint() {
        // safe_val = compute()
        // execute(safe_val)
        let stmts = vec![
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "safe_val".to_string(),
                    source_vars: vec!["compute".to_string()],
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "execute".to_string(),
                    args: vec!["safe_val".to_string()],
                },
                line: 3,
            },
        ];

        let (mut graph, _func_idx) = make_graph_with_cfg("safe_handler", stmts);
        let findings = TaintEngine::analyze_all(&mut graph);

        assert!(findings.is_empty());
    }

    #[test]
    fn test_tainted_parameter() {
        // Function with a parameter named "request" — should be auto-tainted.
        let stmts = vec![
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "data".to_string(),
                    source_vars: vec!["request".to_string()],
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "eval".to_string(),
                    args: vec!["data".to_string()],
                },
                line: 3,
            },
        ];

        let (mut graph, func_idx) = make_graph_with_cfg("handler", stmts);

        // Add a Parameter node.
        let param_idx = graph.graph.add_node(NodeWeight::Parameter {
            name: "request".to_string(),
            function_node: func_idx,
            position: 0,
            is_taint_source: false,
        });
        graph
            .graph
            .add_edge(param_idx, func_idx, EdgeWeight::FlowsTo);

        let findings = TaintEngine::analyze_all(&mut graph);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].sink_name, "eval");

        // Verify the parameter was marked as a taint source.
        if let NodeWeight::Parameter {
            is_taint_source, ..
        } = &graph.graph[param_idx]
        {
            assert!(*is_taint_source);
        } else {
            panic!("expected Parameter node");
        }
    }

    #[test]
    fn test_branch_merge_taint() {
        // Two blocks both flowing into a third block.
        // Block 0 (entry): a = request.body
        // Block 1 (true branch): b = a
        // Block 2 (false branch): (no taint)
        // Block 3 (merge): query(b)  — b is tainted on one path
        let mut cfg_graph = DiGraph::new();

        let mut b0 = BasicBlock::new();
        b0.statements.push(CfgStatement {
            kind: CfgStatementKind::Assignment {
                target: "a".to_string(),
                source_vars: vec!["request.body".to_string()],
            },
            line: 1,
        });
        b0.statements.push(CfgStatement {
            kind: CfgStatementKind::Guard {
                condition_vars: vec!["flag".to_string()],
            },
            line: 2,
        });
        let b0_idx = cfg_graph.add_node(b0);

        let mut b1 = BasicBlock::new();
        b1.statements.push(CfgStatement {
            kind: CfgStatementKind::Assignment {
                target: "b".to_string(),
                source_vars: vec!["a".to_string()],
            },
            line: 3,
        });
        let b1_idx = cfg_graph.add_node(b1);

        let b2 = BasicBlock::new(); // empty — no taint
        let b2_idx = cfg_graph.add_node(b2);

        let mut b3 = BasicBlock::new();
        b3.statements.push(CfgStatement {
            kind: CfgStatementKind::Call {
                name: "query".to_string(),
                args: vec!["b".to_string()],
            },
            line: 5,
        });
        let b3_idx = cfg_graph.add_node(b3);

        cfg_graph.add_edge(b0_idx, b1_idx, CfgEdge::TrueBranch);
        cfg_graph.add_edge(b0_idx, b2_idx, CfgEdge::FalseBranch);
        cfg_graph.add_edge(b1_idx, b3_idx, CfgEdge::Normal);
        cfg_graph.add_edge(b2_idx, b3_idx, CfgEdge::Normal);

        let cfg = FunctionCfg {
            blocks: cfg_graph,
            entry: b0_idx,
            exits: vec![b3_idx],
        };

        let mut graph = CodeGraph::new();
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "test.py".to_string(),
            language: Language::Python,
        });
        graph
            .file_nodes
            .insert("test.py".to_string(), file_idx);

        let func_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "branchy".to_string(),
            kind: SymbolKind::Function,
            file_path: "test.py".to_string(),
            start_line: 1,
            end_line: 10,
            exported: true,
        });
        graph.function_cfgs.insert(func_idx, cfg);

        let findings = TaintEngine::analyze_all(&mut graph);

        // b is tainted on the true branch, so after merge it should still
        // be considered tainted (union semantics).
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].tainted_var, "b");
        assert_eq!(findings[0].sink_name, "query");
    }

    #[test]
    fn test_phi_node_propagation() {
        // a = request.body
        // b = safe_value
        // c = phi(a, b)   — c should be tainted because a is
        // execute(c)
        let stmts = vec![
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "a".to_string(),
                    source_vars: vec!["request.body".to_string()],
                },
                line: 1,
            },
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target: "b".to_string(),
                    source_vars: vec!["safe_value".to_string()],
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::PhiNode {
                    target: "c".to_string(),
                    sources: vec!["a".to_string(), "b".to_string()],
                },
                line: 3,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "execute".to_string(),
                    args: vec!["c".to_string()],
                },
                line: 4,
            },
        ];

        let (mut graph, _) = make_graph_with_cfg("phi_test", stmts);
        let findings = TaintEngine::analyze_all(&mut graph);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].tainted_var, "c");
    }

    #[test]
    fn test_is_source_param() {
        assert!(is_source_param("request"));
        assert!(is_source_param("req"));
        assert!(is_source_param("input"));
        assert!(is_source_param("user_input"));
        assert!(is_source_param("query"));
        assert!(!is_source_param("count"));
        assert!(!is_source_param("result"));
    }

    #[test]
    fn test_topo_order_single_block() {
        let mut blocks = DiGraph::new();
        let entry = blocks.add_node(BasicBlock::new());
        let cfg = FunctionCfg {
            blocks,
            entry,
            exits: vec![entry],
        };
        let order = topo_order_or_bfs(&cfg);
        assert_eq!(order.len(), 1);
        assert_eq!(order[0], entry);
    }

    #[test]
    fn test_topo_order_with_cycle() {
        let mut blocks = DiGraph::new();
        let b0 = blocks.add_node(BasicBlock::new());
        let b1 = blocks.add_node(BasicBlock::new());
        blocks.add_edge(b0, b1, CfgEdge::Normal);
        blocks.add_edge(b1, b0, CfgEdge::Normal); // back edge = cycle

        let cfg = FunctionCfg {
            blocks,
            entry: b0,
            exits: vec![b1],
        };
        let order = topo_order_or_bfs(&cfg);
        // Both blocks should be present despite the cycle.
        assert_eq!(order.len(), 2);
        assert!(order.contains(&b0));
        assert!(order.contains(&b1));
    }
}
