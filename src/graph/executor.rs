//! Graph pipeline executor.
//!
//! Takes a sequence of [`GraphStage`] steps, a [`CodeGraph`] reference, and optional
//! seed nodes, then executes the pipeline to produce either [`AuditFinding`]s (when the
//! last stage is `Flag`) or [`QueryResult`]s (otherwise).

use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::Direction;
use petgraph::algo::tarjan_scc;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::audit::models::AuditFinding;
use crate::audit::pipelines::helpers::{
    is_barrel_file, is_excluded_for_arch_analysis, is_test_file,
};
use crate::graph::pipeline::{
    EdgeType, GraphStage, MetricValue, PipelineNode, interpolate_message,
};
use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};
use crate::query_engine::QueryResult;

// ---------------------------------------------------------------------------
// PipelineOutput
// ---------------------------------------------------------------------------

/// The result of executing a graph pipeline.
pub enum PipelineOutput {
    Findings(Vec<AuditFinding>),
    Results(Vec<QueryResult>),
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Alias for [`run_pipeline`]. Prefer calling [`run_pipeline`] directly.
/// Kept for backward compatibility.
pub fn execute_graph_pipeline(
    stages: &[GraphStage],
    graph: &CodeGraph,
    seed_nodes: Option<Vec<NodeIndex>>,
    pipeline_name: &str,
) -> anyhow::Result<PipelineOutput> {
    run_pipeline(stages, graph, seed_nodes, pipeline_name)
}

// ---------------------------------------------------------------------------
// Real entry point — clean design avoiding the "last is flag" ambiguity
// ---------------------------------------------------------------------------

/// Execute a graph pipeline against a `CodeGraph`.
///
/// This is the canonical implementation. The function above is a wrapper that
/// calls this one.
pub fn run_pipeline(
    stages: &[GraphStage],
    graph: &CodeGraph,
    seed_nodes: Option<Vec<NodeIndex>>,
    pipeline_name: &str,
) -> anyhow::Result<PipelineOutput> {
    // Helper closures for WhereClause::eval
    let is_test_fn = |path: &str| is_test_file(path);
    let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
    let is_barrel_fn = |path: &str| is_barrel_file(path);

    // Determine if last stage is Flag
    let (pipeline_stages, flag_stage) = if let Some(GraphStage::Flag { flag }) = stages.last() {
        (&stages[..stages.len() - 1], Some(flag))
    } else {
        (stages, None)
    };

    // Start with seed nodes or empty
    let mut nodes: Vec<PipelineNode> = match seed_nodes {
        Some(idxs) => idxs
            .into_iter()
            .filter_map(|idx| pipeline_node_from_index(idx, graph))
            .collect(),
        None => Vec::new(),
    };

    // Execute all non-Flag stages
    for stage in pipeline_stages {
        nodes = execute_stage(
            stage,
            nodes,
            graph,
            pipeline_name,
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
        )?;
    }

    // If last stage was Flag, produce findings
    if let Some(flag) = flag_stage {
        let effective_pipeline = flag.pipeline_name.as_deref().unwrap_or(pipeline_name);
        let findings = nodes
            .iter()
            .map(|node| {
                let severity = flag.resolve_severity(node);
                let message = interpolate_message(&flag.message, node);
                AuditFinding {
                    file_path: node.file_path.clone(),
                    line: node.line,
                    column: 1,
                    severity,
                    pipeline: effective_pipeline.to_string(),
                    pattern: flag.pattern.clone(),
                    message,
                    snippet: String::new(),
                }
            })
            .collect();
        Ok(PipelineOutput::Findings(findings))
    } else {
        let results = nodes
            .into_iter()
            .map(pipeline_node_to_query_result)
            .collect();
        Ok(PipelineOutput::Results(results))
    }
}

// ---------------------------------------------------------------------------
// Stage dispatch
// ---------------------------------------------------------------------------

fn execute_stage(
    stage: &GraphStage,
    nodes: Vec<PipelineNode>,
    graph: &CodeGraph,
    _pipeline_name: &str,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<PipelineNode>> {
    match stage {
        GraphStage::Select { select, filter, exclude } => {
            execute_select(select, filter.as_ref(), exclude.as_ref(), graph, is_test_fn, is_generated_fn, is_barrel_fn)
        }
        GraphStage::GroupBy { group_by } => {
            Ok(execute_group_by(group_by, nodes))
        }
        GraphStage::Count { count } => {
            Ok(execute_count(&count.threshold, nodes))
        }
        GraphStage::FindCycles { find_cycles } => {
            execute_find_cycles(&find_cycles.edge, nodes, graph)
        }
        GraphStage::MaxDepth { max_depth } => {
            execute_max_depth(max_depth, nodes, graph)
        }
        GraphStage::Ratio { ratio } => {
            execute_ratio(ratio, nodes, is_test_fn, is_generated_fn, is_barrel_fn)
        }
        GraphStage::Flag { .. } => {
            // Flag is handled at the top level in run_pipeline; if it appears mid-pipeline
            // just pass nodes through unchanged.
            Ok(nodes)
        }
        // Stubs for stages not needed by architecture pipelines
        GraphStage::Traverse { .. } => {
            // TODO: implement BFS traversal
            Ok(nodes)
        }
        GraphStage::Filter { .. } => {
            // TODO: implement edge-based filter
            Ok(nodes)
        }
        GraphStage::MatchName { .. } => {
            // TODO: implement name matching
            Ok(nodes)
        }
        GraphStage::CountEdges { .. } => {
            // TODO: implement edge counting
            Ok(nodes)
        }
        GraphStage::Pair { .. } => {
            // TODO: implement acquire/release pairing
            Ok(nodes)
        }
    }
}

// ---------------------------------------------------------------------------
// Select stage
// ---------------------------------------------------------------------------

fn execute_select(
    node_type: &crate::graph::pipeline::NodeType,
    filter: Option<&crate::graph::pipeline::WhereClause>,
    exclude: Option<&crate::graph::pipeline::WhereClause>,
    graph: &CodeGraph,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<PipelineNode>> {
    use crate::graph::pipeline::NodeType;

    let mut result = Vec::new();

    match node_type {
        NodeType::File => {
            for (path, &file_idx) in &graph.file_nodes {
                let node = PipelineNode {
                    node_idx: file_idx,
                    file_path: path.clone(),
                    name: path.clone(),
                    kind: "file".to_string(),
                    line: 1,
                    exported: false,
                    language: match &graph.graph[file_idx] {
                        NodeWeight::File { language, .. } => language.as_str().to_string(),
                        _ => String::new(),
                    },
                    metrics: HashMap::new(),
                };
                // Apply where clause
                if let Some(wc) = filter {
                    if !wc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn) {
                        continue;
                    }
                }
                // Apply exclude clause
                if let Some(exc) = exclude {
                    if exc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn) {
                        continue;
                    }
                }
                result.push(node);
            }
        }
        NodeType::Symbol => {
            for idx in graph.graph.node_indices() {
                if let NodeWeight::Symbol {
                    name,
                    kind,
                    file_path,
                    start_line,
                    exported,
                    ..
                } = &graph.graph[idx]
                {
                    let language = graph
                        .file_nodes
                        .get(file_path)
                        .and_then(|&file_idx| match &graph.graph[file_idx] {
                            NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                            _ => None,
                        })
                        .unwrap_or_default();

                    let node = PipelineNode {
                        node_idx: idx,
                        file_path: file_path.clone(),
                        name: name.clone(),
                        kind: kind.to_string(),
                        line: *start_line,
                        exported: *exported,
                        language,
                        metrics: HashMap::new(),
                    };
                    // Apply where clause
                    if let Some(wc) = filter {
                        if !wc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn) {
                            continue;
                        }
                    }
                    // Apply exclude clause
                    if let Some(exc) = exclude {
                        if exc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn) {
                            continue;
                        }
                    }
                    result.push(node);
                }
            }
        }
        NodeType::CallSite => {
            for idx in graph.graph.node_indices() {
                if let NodeWeight::CallSite {
                    name,
                    file_path,
                    line,
                } = &graph.graph[idx]
                {
                    let language = graph
                        .file_nodes
                        .get(file_path)
                        .and_then(|&file_idx| match &graph.graph[file_idx] {
                            NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                            _ => None,
                        })
                        .unwrap_or_default();

                    let node = PipelineNode {
                        node_idx: idx,
                        file_path: file_path.clone(),
                        name: name.clone(),
                        kind: "callsite".to_string(),
                        line: *line,
                        exported: false,
                        language,
                        metrics: HashMap::new(),
                    };
                    // Apply where clause
                    if let Some(wc) = filter {
                        if !wc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn) {
                            continue;
                        }
                    }
                    // Apply exclude clause
                    if let Some(exc) = exclude {
                        if exc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn) {
                            continue;
                        }
                    }
                    result.push(node);
                }
            }
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// GroupBy stage
// ---------------------------------------------------------------------------

/// Tag each node with `metrics["_group"] = MetricValue::Text(group_key)`.
fn execute_group_by(group_by_field: &str, mut nodes: Vec<PipelineNode>) -> Vec<PipelineNode> {
    for node in &mut nodes {
        let group_key = match group_by_field {
            "file_path" | "file" => node.file_path.clone(),
            "language" => node.language.clone(),
            "kind" => node.kind.clone(),
            "name" => node.name.clone(),
            other => {
                // Check metrics for the key
                node.metrics
                    .get(other)
                    .map(|v| v.as_str().to_string())
                    .unwrap_or_default()
            }
        };
        node.metrics
            .insert("_group".to_string(), MetricValue::Text(group_key));
    }
    nodes
}

// ---------------------------------------------------------------------------
// Count stage
// ---------------------------------------------------------------------------

/// Group nodes by `_group` metric, count members per group, keep groups
/// whose count satisfies the threshold predicate. Emits one representative
/// `PipelineNode` per surviving group with `metrics["count"]` set.
fn execute_count(
    threshold: &crate::graph::pipeline::NumericPredicate,
    nodes: Vec<PipelineNode>,
) -> Vec<PipelineNode> {
    // Collect groups preserving insertion order for determinism
    let mut group_order: Vec<String> = Vec::new();
    let mut group_map: HashMap<String, Vec<PipelineNode>> = HashMap::new();

    for node in nodes {
        let group_key = node.metric_str("_group").to_string();
        if !group_map.contains_key(&group_key) {
            group_order.push(group_key.clone());
        }
        group_map.entry(group_key).or_default().push(node);
    }

    let mut result = Vec::new();
    for key in &group_order {
        let members = &group_map[key];
        let count = members.len() as f64;
        if !threshold.matches(count) {
            continue;
        }
        // Pick a representative (first member) and set count metric
        let mut rep = members[0].clone();
        rep.metrics
            .insert("count".to_string(), MetricValue::Int(members.len() as i64));
        result.push(rep);
    }
    result
}

// ---------------------------------------------------------------------------
// FindCycles stage
// ---------------------------------------------------------------------------

fn execute_find_cycles(
    edge_type: &EdgeType,
    nodes: Vec<PipelineNode>,
    graph: &CodeGraph,
) -> anyhow::Result<Vec<PipelineNode>> {
    // Build a subgraph containing only the nodes in our pipeline set,
    // connected by edges of the specified type.
    let node_set: HashSet<NodeIndex> = nodes.iter().map(|n| n.node_idx).collect();
    let edge_weight_matches = |ew: &EdgeWeight| edge_matches_type(ew, edge_type);

    // sub_idx -> original NodeIndex
    let mut import_graph = petgraph::graph::DiGraph::<NodeIndex, ()>::new();
    let mut orig_to_sub: HashMap<NodeIndex, NodeIndex> = HashMap::new();

    for &orig_idx in &node_set {
        let sub_idx = import_graph.add_node(orig_idx);
        orig_to_sub.insert(orig_idx, sub_idx);
    }

    for edge in graph.graph.edge_references() {
        if !edge_weight_matches(edge.weight()) {
            continue;
        }
        let src = edge.source();
        let tgt = edge.target();
        if let (Some(&sub_src), Some(&sub_tgt)) =
            (orig_to_sub.get(&src), orig_to_sub.get(&tgt))
        {
            import_graph.add_edge(sub_src, sub_tgt, ());
        }
    }

    // Run Tarjan's SCC
    let sccs = tarjan_scc(&import_graph);
    let mut result = Vec::new();

    for scc in sccs {
        if scc.len() < 2 {
            continue;
        }

        // Map sub-indices back to original NodeIndex values
        let orig_indices: Vec<NodeIndex> = scc.iter().map(|&sub| import_graph[sub]).collect();

        // Collect file paths for cycle members
        let participants: Vec<String> = orig_indices
            .iter()
            .filter_map(|&idx| {
                match &graph.graph[idx] {
                    NodeWeight::File { path, .. } => Some(path.clone()),
                    NodeWeight::Symbol { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                }
            })
            .collect();

        let cycle_size = scc.len();
        let scc_set: HashSet<NodeIndex> = orig_indices.iter().copied().collect();
        let cycle_path = ordered_cycle_path_for_edge(
            &orig_indices,
            &scc_set,
            graph,
            edge_type,
        );

        // Lexicographically smallest file path as representative
        let representative_path = participants.iter().min().cloned().unwrap_or_default();
        let rep_node_idx = orig_indices
            .iter()
            .find(|&&idx| {
                let path = node_path(&graph.graph[idx]);
                path == representative_path
            })
            .copied()
            .unwrap_or(orig_indices[0]);

        let rep_base = pipeline_node_from_index(rep_node_idx, graph)
            .unwrap_or_else(|| PipelineNode {
                node_idx: rep_node_idx,
                file_path: representative_path.clone(),
                name: representative_path.clone(),
                kind: "file".to_string(),
                line: 1,
                exported: false,
                language: String::new(),
                metrics: HashMap::new(),
            });

        let mut rep = PipelineNode {
            node_idx: rep_node_idx,
            file_path: representative_path.clone(),
            name: representative_path,
            kind: "cycle".to_string(),
            line: rep_base.line,
            exported: false,
            language: rep_base.language,
            metrics: HashMap::new(),
        };
        rep.metrics
            .insert("cycle_size".to_string(), MetricValue::Int(cycle_size as i64));
        rep.metrics
            .insert("cycle_path".to_string(), MetricValue::Text(cycle_path));

        result.push(rep);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// MaxDepth stage
// ---------------------------------------------------------------------------

fn execute_max_depth(
    config: &crate::graph::pipeline::MaxDepthConfig,
    nodes: Vec<PipelineNode>,
    graph: &CodeGraph,
) -> anyhow::Result<Vec<PipelineNode>> {
    let skip_barrels = config.skip_barrel_files.unwrap_or(false);
    let edge_type = &config.edge;

    // The nodes in the pipeline form the working set. Compute BFS depth
    // from source nodes (in-degree == 0 within the set).
    let node_set: HashSet<NodeIndex> = nodes.iter().map(|n| n.node_idx).collect();
    let node_by_idx: HashMap<NodeIndex, &PipelineNode> =
        nodes.iter().map(|n| (n.node_idx, n)).collect();

    // Compute in-degree within the subgraph (edges of our type between nodes in the set)
    let mut in_degree: HashMap<NodeIndex, usize> =
        node_set.iter().map(|&idx| (idx, 0)).collect();

    for edge in graph.graph.edge_references() {
        if !edge_matches_type(edge.weight(), edge_type) {
            continue;
        }
        let src = edge.source();
        let tgt = edge.target();
        if node_set.contains(&src) && node_set.contains(&tgt) {
            *in_degree.entry(tgt).or_insert(0) += 1;
        }
    }

    // BFS from roots (in-degree == 0)
    let roots: VecDeque<NodeIndex> = in_degree
        .iter()
        .filter(|&(_, deg)| *deg == 0)
        .map(|(&idx, _)| idx)
        .collect();

    // depth_map: NodeIndex -> depth
    let mut depth_map: HashMap<NodeIndex, usize> =
        node_set.iter().map(|&idx| (idx, 0)).collect();

    // Topological BFS/DP (same pattern as dependency_depth.rs)
    // Build topological order via Kahn's
    let mut kahn_in_degree = in_degree.clone();
    let mut queue: VecDeque<NodeIndex> = roots;
    let mut topo_order: Vec<NodeIndex> = Vec::new();
    let mut visited_kahn: HashSet<NodeIndex> = HashSet::new();

    while let Some(node) = queue.pop_front() {
        if !visited_kahn.insert(node) {
            continue;
        }
        topo_order.push(node);
        for edge in graph.graph.edges_directed(node, Direction::Outgoing) {
            if !edge_matches_type(edge.weight(), edge_type) {
                continue;
            }
            let tgt = edge.target();
            if !node_set.contains(&tgt) {
                continue;
            }
            if let Some(deg) = kahn_in_degree.get_mut(&tgt) {
                *deg = deg.saturating_sub(1);
                if *deg == 0 && !visited_kahn.contains(&tgt) {
                    queue.push_back(tgt);
                }
            }
        }
    }

    // Append cycle nodes (not visited)
    for &idx in &node_set {
        if !visited_kahn.contains(&idx) {
            topo_order.push(idx);
        }
    }

    // DP relaxation in topological order
    for &node in &topo_order {
        let node_depth = depth_map.get(&node).copied().unwrap_or(0);
        let hop: usize = if skip_barrels {
            let path = match node_by_idx.get(&node) {
                Some(n) => n.file_path.as_str(),
                None => "",
            };
            if is_barrel_file(path) { 0 } else { 1 }
        } else {
            1
        };

        for edge in graph.graph.edges_directed(node, Direction::Outgoing) {
            if !edge_matches_type(edge.weight(), edge_type) {
                continue;
            }
            let tgt = edge.target();
            if !node_set.contains(&tgt) {
                continue;
            }
            let candidate = node_depth + hop;
            let entry = depth_map.entry(tgt).or_insert(0);
            if candidate > *entry {
                *entry = candidate;
            }
        }
    }

    // Filter nodes meeting the threshold
    let mut result = Vec::new();
    for node in nodes {
        let depth = depth_map.get(&node.node_idx).copied().unwrap_or(0);
        if config.threshold.matches(depth as f64) {
            let mut n = node;
            n.metrics
                .insert("depth".to_string(), MetricValue::Int(depth as i64));
            result.push(n);
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Ratio stage
// ---------------------------------------------------------------------------

fn execute_ratio(
    config: &crate::graph::pipeline::RatioConfig,
    nodes: Vec<PipelineNode>,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<PipelineNode>> {
    // Group by `_group` metric
    let mut group_order: Vec<String> = Vec::new();
    let mut group_map: HashMap<String, Vec<PipelineNode>> = HashMap::new();

    for node in nodes {
        let group_key = node.metric_str("_group").to_string();
        if !group_map.contains_key(&group_key) {
            group_order.push(group_key.clone());
        }
        group_map.entry(group_key).or_default().push(node);
    }

    let mut result = Vec::new();

    for key in &group_order {
        let members = &group_map[key];

        // Count numerator and denominator
        let numerator_count = members
            .iter()
            .filter(|n| {
                if let Some(wc) = &config.numerator.filter {
                    wc.eval(n, is_test_fn, is_generated_fn, is_barrel_fn)
                } else {
                    true
                }
            })
            .count();

        let denominator_count = members
            .iter()
            .filter(|n| {
                if let Some(wc) = &config.denominator.filter {
                    wc.eval(n, is_test_fn, is_generated_fn, is_barrel_fn)
                } else {
                    true
                }
            })
            .count();

        if denominator_count == 0 {
            continue;
        }

        let ratio = numerator_count as f64 / denominator_count as f64;

        // Apply threshold where clause (evaluates against a node with ratio metric set)
        let mut rep = members[0].clone();
        rep.metrics
            .insert("count".to_string(), MetricValue::Int(numerator_count as i64));
        rep.metrics
            .insert("ratio".to_string(), MetricValue::Float(ratio));

        if let Some(wc) = &config.threshold {
            if !wc.eval_metrics(&rep) {
                continue;
            }
        }

        result.push(rep);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a `NodeIndex` to a `PipelineNode`, returning `None` for
/// unsupported node types (Parameter, ExternalSource).
pub fn pipeline_node_from_index(idx: NodeIndex, graph: &CodeGraph) -> Option<PipelineNode> {
    match &graph.graph[idx] {
        NodeWeight::File { path, language } => Some(PipelineNode {
            node_idx: idx,
            file_path: path.clone(),
            name: path.clone(),
            kind: "file".to_string(),
            line: 1,
            exported: false,
            language: language.as_str().to_string(),
            metrics: HashMap::new(),
        }),
        NodeWeight::Symbol {
            name,
            kind,
            file_path,
            start_line,
            exported,
            ..
        } => {
            let language = graph
                .file_nodes
                .get(file_path)
                .and_then(|&file_idx| match &graph.graph[file_idx] {
                    NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            Some(PipelineNode {
                node_idx: idx,
                file_path: file_path.clone(),
                name: name.clone(),
                kind: kind.to_string(),
                line: *start_line,
                exported: *exported,
                language,
                metrics: HashMap::new(),
            })
        }
        NodeWeight::CallSite {
            name,
            file_path,
            line,
        } => {
            let language = graph
                .file_nodes
                .get(file_path)
                .and_then(|&file_idx| match &graph.graph[file_idx] {
                    NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            Some(PipelineNode {
                node_idx: idx,
                file_path: file_path.clone(),
                name: name.clone(),
                kind: "callsite".to_string(),
                line: *line,
                exported: false,
                language,
                metrics: HashMap::new(),
            })
        }
        NodeWeight::Parameter { .. } | NodeWeight::ExternalSource { .. } => None,
    }
}

/// Convert a `PipelineNode` to a `QueryResult` for non-Flag pipeline output.
fn pipeline_node_to_query_result(node: PipelineNode) -> QueryResult {
    QueryResult {
        name: node.name,
        kind: node.kind,
        file: node.file_path,
        line: node.line,
        end_line: node.line,
        column: 1,
        exported: node.exported,
        signature: None,
        docstring: None,
        body: None,
        preview: None,
        parent: None,
    }
}

/// Check if an EdgeWeight matches an EdgeType.
fn edge_matches_type(ew: &EdgeWeight, et: &EdgeType) -> bool {
    match (ew, et) {
        (EdgeWeight::Calls, EdgeType::Calls) => true,
        (EdgeWeight::Imports, EdgeType::Imports) => true,
        (EdgeWeight::FlowsTo, EdgeType::FlowsTo) => true,
        (EdgeWeight::Acquires { .. }, EdgeType::Acquires) => true,
        (EdgeWeight::ReleasedBy, EdgeType::ReleasedBy) => true,
        (EdgeWeight::Contains, EdgeType::Contains) => true,
        (EdgeWeight::Exports, EdgeType::Exports) => true,
        (EdgeWeight::DefinedIn, EdgeType::DefinedIn) => true,
        _ => false,
    }
}

/// Extract a display path from a NodeWeight.
fn node_path(nw: &NodeWeight) -> String {
    match nw {
        NodeWeight::File { path, .. } => path.clone(),
        NodeWeight::Symbol { file_path, .. } => file_path.clone(),
        NodeWeight::CallSite { file_path, .. } => file_path.clone(),
        _ => String::new(),
    }
}

/// Walk the SCC members via DFS along edges of the given type to produce an ordered
/// cycle path string like `"a.rs -> b.rs -> c.rs -> a.rs"`.
/// Falls back to sorted member paths if no Hamiltonian cycle walk is found.
fn ordered_cycle_path_for_edge(
    members: &[NodeIndex],
    scc_set: &HashSet<NodeIndex>,
    graph: &CodeGraph,
    edge_type: &EdgeType,
) -> String {
    if members.is_empty() {
        return String::new();
    }
    let start = members[0];
    let mut path: Vec<NodeIndex> = vec![start];
    let mut visited: HashSet<NodeIndex> = HashSet::from([start]);

    'outer: loop {
        let current = *path.last().unwrap();
        for edge in graph.graph.edges_directed(current, Direction::Outgoing) {
            if !edge_matches_type(edge.weight(), edge_type) {
                continue;
            }
            let next = edge.target();
            if !scc_set.contains(&next) {
                continue;
            }
            if next == start && path.len() > 1 {
                path.push(start);
                break 'outer;
            }
            if !visited.contains(&next) {
                visited.insert(next);
                path.push(next);
                continue 'outer;
            }
        }
        // Couldn't complete a simple Hamiltonian cycle — fall back to sorted paths
        let mut sorted: Vec<String> = members
            .iter()
            .map(|&idx| node_path(&graph.graph[idx]))
            .filter(|p| !p.is_empty())
            .collect();
        sorted.sort();
        return sorted.join(" -> ");
    }

    path.iter()
        .map(|&idx| node_path(&graph.graph[idx]))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" -> ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::pipeline::{
        CountConfig, EdgeType, FlagConfig, FindCyclesConfig, GraphStage, MaxDepthConfig,
        NodeType, NumericPredicate, RatioConfig, NumeratorConfig, DenominatorConfig,
        WhereClause,
    };
    use crate::language::Language;

    // ── Test graph builders ──────────────────────────────────────────

    fn make_file_graph(paths: &[&str]) -> CodeGraph {
        let mut g = CodeGraph::new();
        for path in paths {
            let idx = g.graph.add_node(NodeWeight::File {
                path: path.to_string(),
                language: Language::Rust,
            });
            g.file_nodes.insert(path.to_string(), idx);
        }
        g
    }

    fn add_import(g: &mut CodeGraph, from: &str, to: &str) {
        let from_idx = *g.file_nodes.get(from).unwrap();
        let to_idx = *g.file_nodes.get(to).unwrap();
        g.graph.add_edge(from_idx, to_idx, EdgeWeight::Imports);
    }

    // ── Test 1: select(file) with a 2-file graph ─────────────────────

    #[test]
    fn test_select_file_nodes() {
        let graph = make_file_graph(&["src/a.rs", "src/b.rs"]);
        let stages = vec![GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        }];
        let out = run_pipeline(&stages, &graph, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 2);
                let files: HashSet<_> = results.iter().map(|r| r.file.as_str()).collect();
                assert!(files.contains("src/a.rs"));
                assert!(files.contains("src/b.rs"));
            }
            _ => panic!("expected Results"),
        }
    }

    #[test]
    fn test_select_with_exclude() {
        let graph = make_file_graph(&["src/a.rs", "src/a_test.rs"]);
        let stages = vec![GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: Some(WhereClause {
                is_test_file: Some(true),
                ..Default::default()
            }),
        }];
        let out = run_pipeline(&stages, &graph, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].file, "src/a.rs");
            }
            _ => panic!("expected Results"),
        }
    }

    // ── Test 2: find_cycles with a 2-node cycle ─────────────────────

    #[test]
    fn test_find_cycles_two_node_cycle() {
        let mut graph = make_file_graph(&["a.rs", "b.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "a.rs");

        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::FindCycles {
                find_cycles: FindCyclesConfig { edge: EdgeType::Imports },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 1, "expected exactly 1 cycle node");
                assert_eq!(results[0].kind, "cycle");
            }
            _ => panic!("expected Results"),
        }
    }

    #[test]
    fn test_find_cycles_two_node_cycle_size_metric() {
        let mut graph = make_file_graph(&["a.rs", "b.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "a.rs");

        // Run without Flag to inspect pipeline nodes directly
        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);

        // Run select stage manually
        let select_stage = GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        };
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
        ).unwrap();

        let cycle_stage = GraphStage::FindCycles {
            find_cycles: FindCyclesConfig { edge: EdgeType::Imports },
        };
        let cycle_nodes = execute_stage(
            &cycle_stage,
            nodes,
            &graph,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
        ).unwrap();

        assert_eq!(cycle_nodes.len(), 1);
        assert_eq!(cycle_nodes[0].metric_f64("cycle_size") as usize, 2);
    }

    // ── Test 3: find_cycles with a DAG returns empty ─────────────────

    #[test]
    fn test_find_cycles_dag_returns_empty() {
        let mut graph = make_file_graph(&["a.rs", "b.rs", "c.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "c.rs");

        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::FindCycles {
                find_cycles: FindCyclesConfig { edge: EdgeType::Imports },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert!(results.is_empty(), "DAG should produce no cycles");
            }
            _ => panic!("expected Results"),
        }
    }

    // ── Test 4: group_by + count with threshold ───────────────────────

    #[test]
    fn test_group_by_count_threshold() {
        use crate::models::SymbolKind;

        let mut graph = make_file_graph(&["src/a.rs", "src/b.rs"]);

        // Add 3 symbols to a.rs and 1 to b.rs
        let a_idx = *graph.file_nodes.get("src/a.rs").unwrap();
        let b_idx = *graph.file_nodes.get("src/b.rs").unwrap();

        for i in 0..3u32 {
            let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
                name: format!("fn_a_{}", i),
                kind: SymbolKind::Function,
                file_path: "src/a.rs".to_string(),
                start_line: i + 1,
                end_line: i + 5,
                exported: true,
            });
            graph.graph.add_edge(a_idx, sym_idx, EdgeWeight::Contains);
        }

        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "fn_b".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/b.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });
        graph.graph.add_edge(b_idx, sym_idx, EdgeWeight::Contains);

        let stages = vec![
            GraphStage::Select {
                select: NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::GroupBy {
                group_by: "file_path".to_string(),
            },
            GraphStage::Count {
                count: CountConfig {
                    threshold: NumericPredicate {
                        gte: Some(2.0),
                        ..Default::default()
                    },
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                // Only src/a.rs has >= 2 symbols
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].file, "src/a.rs");
            }
            _ => panic!("expected Results"),
        }
    }

    // ── Test 5: max_depth returns correct depth ───────────────────────

    #[test]
    fn test_max_depth_three_level_chain() {
        // a.rs -> b.rs -> c.rs  (depth: a=0, b=1, c=2)
        let mut graph = make_file_graph(&["a.rs", "b.rs", "c.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "c.rs");

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);

        // Select all files first
        let select_stage = GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        };
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
        ).unwrap();

        // Run max_depth with threshold >= 2 (should match c.rs only)
        let max_depth_stage = GraphStage::MaxDepth {
            max_depth: MaxDepthConfig {
                edge: EdgeType::Imports,
                skip_barrel_files: Some(false),
                threshold: NumericPredicate {
                    gte: Some(2.0),
                    ..Default::default()
                },
            },
        };
        let result = execute_stage(
            &max_depth_stage,
            nodes,
            &graph,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
        ).unwrap();

        assert_eq!(result.len(), 1, "only c.rs should have depth >= 2");
        assert_eq!(result[0].file_path, "c.rs");
        assert_eq!(result[0].metric_f64("depth") as usize, 2);
    }

    // ── Test 6: flag stage produces AuditFinding with correct severity ─

    #[test]
    fn test_flag_stage_produces_findings() {
        let graph = make_file_graph(&["src/big.rs"]);

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);

        // Create a node with count=25
        let select_stage = GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        };
        let mut nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            "test_pipeline",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
        ).unwrap();

        // Inject a count metric manually
        for n in &mut nodes {
            n.metrics.insert("count".to_string(), MetricValue::Int(25));
        }

        let flag_config = FlagConfig {
            pattern: "oversized_module".to_string(),
            message: "{{file}} has {{count}} symbols".to_string(),
            severity: None,
            severity_map: Some(vec![
                crate::graph::pipeline::SeverityEntry {
                    when: Some(WhereClause {
                        count: Some(NumericPredicate {
                            gte: Some(20.0),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                crate::graph::pipeline::SeverityEntry {
                    when: None,
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };

        let effective_pipeline = flag_config.pipeline_name.as_deref().unwrap_or("test_pipeline");
        let findings: Vec<AuditFinding> = nodes
            .iter()
            .map(|node| {
                let severity = flag_config.resolve_severity(node);
                let message = interpolate_message(&flag_config.message, node);
                AuditFinding {
                    file_path: node.file_path.clone(),
                    line: node.line,
                    column: 1,
                    severity,
                    pipeline: effective_pipeline.to_string(),
                    pattern: flag_config.pattern.clone(),
                    message,
                    snippet: String::new(),
                }
            })
            .collect();

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error"); // count=25 >= 20
        assert_eq!(findings[0].pattern, "oversized_module");
        assert_eq!(findings[0].pipeline, "test_pipeline");
        assert!(findings[0].message.contains("src/big.rs"));
        assert!(findings[0].message.contains("25"));
    }

    #[test]
    fn test_run_pipeline_flag_produces_findings() {
        let graph = make_file_graph(&["src/large.rs"]);

        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::Flag {
                flag: FlagConfig {
                    pattern: "test_pattern".to_string(),
                    message: "Found {{file}}".to_string(),
                    severity: Some("info".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];

        let out = run_pipeline(&stages, &graph, None, "my_pipeline").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert_eq!(findings.len(), 1);
                assert_eq!(findings[0].severity, "info");
                assert_eq!(findings[0].pattern, "test_pattern");
                assert_eq!(findings[0].pipeline, "my_pipeline");
            }
            _ => panic!("expected Findings"),
        }
    }

    // ── Test 7: execute_graph_pipeline delegates to run_pipeline ────────

    #[test]
    fn test_execute_graph_pipeline_wrapper() {
        // execute_graph_pipeline must delegate to run_pipeline.
        // A full pipeline ending in Flag should produce non-empty Findings
        // with the correct pipeline name and pattern.
        let graph = make_file_graph(&["src/x.rs"]);
        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::Flag {
                flag: FlagConfig {
                    pattern: "wrapper_pattern".to_string(),
                    message: "Found {{file}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];

        let out = execute_graph_pipeline(&stages, &graph, None, "wrapper_pipeline").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(!findings.is_empty(), "expected at least one finding");
                assert_eq!(findings[0].pipeline, "wrapper_pipeline");
                assert_eq!(findings[0].pattern, "wrapper_pattern");
                assert_eq!(findings[0].severity, "warning");
            }
            _ => panic!("expected Findings, not Results"),
        }
    }

    // ── Test 8: ratio stage ──────────────────────────────────────────

    #[test]
    fn test_ratio_exported_symbols() {
        use crate::models::SymbolKind;

        let mut graph = make_file_graph(&["src/a.rs"]);
        let a_idx = *graph.file_nodes.get("src/a.rs").unwrap();

        // 3 exported, 1 not exported
        for i in 0..3u32 {
            let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
                name: format!("pub_fn_{}", i),
                kind: SymbolKind::Function,
                file_path: "src/a.rs".to_string(),
                start_line: i + 1,
                end_line: i + 5,
                exported: true,
            });
            graph.graph.add_edge(a_idx, sym_idx, EdgeWeight::Contains);
        }
        let priv_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "priv_fn".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/a.rs".to_string(),
            start_line: 10,
            end_line: 15,
            exported: false,
        });
        graph.graph.add_edge(a_idx, priv_idx, EdgeWeight::Contains);

        let stages = vec![
            GraphStage::Select {
                select: NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::GroupBy {
                group_by: "file_path".to_string(),
            },
            GraphStage::Ratio {
                ratio: RatioConfig {
                    numerator: NumeratorConfig {
                        filter: Some(WhereClause {
                            exported: Some(true),
                            ..Default::default()
                        }),
                    },
                    denominator: DenominatorConfig {
                        filter: None,
                    },
                    threshold: Some(WhereClause {
                        ratio: Some(NumericPredicate {
                            gte: Some(0.5),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                // 3/4 = 0.75 >= 0.5, so should pass
                assert_eq!(results.len(), 1);
            }
            _ => panic!("expected Results"),
        }
    }
}
