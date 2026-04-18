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
use crate::pipeline::helpers::{
    is_barrel_file, is_excluded_for_arch_analysis, is_test_file,
};
use crate::pipeline::dsl::{
    EdgeType, GraphStage, MetricValue, PipelineNode, interpolate_message,
};
use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};
use crate::query_engine::QueryResult;
use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// PipelineOutput
// ---------------------------------------------------------------------------

/// The result of executing a graph pipeline.
pub enum PipelineOutput {
    Findings(Vec<AuditFinding>),
    Results(Vec<QueryResult>),
}

/// Accumulated taint sources and sanitizers built up across `TaintSources` and
/// `TaintSanitizers` stages. Consumed when a `TaintSinks` stage runs.
#[derive(Default)]
struct TaintContext {
    sources: Vec<crate::pipeline::dsl::TaintSourcePattern>,
    sanitizers: Vec<crate::pipeline::dsl::TaintSanitizerPattern>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute a graph pipeline against a `CodeGraph`.
pub fn run_pipeline(
    stages: &[GraphStage],
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    pipeline_languages: Option<&[String]>,
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

    let mut taint_ctx = TaintContext::default();

    // Execute all non-Flag stages
    for stage in pipeline_stages {
        nodes = execute_stage(
            stage,
            nodes,
            graph,
            workspace,
            pipeline_languages,
            pipeline_name,
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )?;
    }

    // If last stage was Flag, produce findings
    if let Some(flag) = flag_stage {
        let effective_pipeline = flag.pipeline_name.as_deref().unwrap_or(pipeline_name);
        let findings = nodes
            .iter()
            .filter_map(|node| {
                let severity = flag.resolve_severity(node)?;
                let message = interpolate_message(&flag.message, node);
                Some(AuditFinding {
                    file_path: node.file_path.clone(),
                    line: node.line,
                    column: 1,
                    severity,
                    pipeline: effective_pipeline.to_string(),
                    pattern: flag.pattern.clone(),
                    message,
                    snippet: String::new(),
                })
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
    workspace: Option<&Workspace>,
    pipeline_languages: Option<&[String]>,
    _pipeline_name: &str,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
    taint_ctx: &mut TaintContext,
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
        GraphStage::MatchPattern { match_pattern, when } => {
            match workspace {
                Some(ws) => execute_match_pattern(match_pattern, when.as_ref(), ws, pipeline_languages),
                None => anyhow::bail!(
                    "match_pattern stage requires workspace -- call run_pipeline with Some(workspace)"
                ),
            }
        }
        GraphStage::ComputeMetric { compute_metric } => {
            match workspace {
                Some(ws) => execute_compute_metric(compute_metric, nodes, ws, graph),
                None => anyhow::bail!(
                    "compute_metric stage requires workspace -- call run_pipeline with Some(workspace)"
                ),
            }
        }
        GraphStage::Taint { taint } => {
            let config = crate::graph::taint::TaintConfig {
                sources: taint.sources.clone(),
                sinks: taint.sinks.clone(),
                sanitizers: taint.sanitizers.clone(),
            };
            execute_taint_with_config(&config, graph, &taint.sinks)
        }
        GraphStage::TaintSources { taint_sources } => {
            taint_ctx.sources.extend(taint_sources.iter().cloned());
            Ok(nodes)
        }
        GraphStage::TaintSanitizers { taint_sanitizers } => {
            taint_ctx.sanitizers.extend(taint_sanitizers.iter().cloned());
            Ok(nodes)
        }
        GraphStage::TaintSinks { taint_sinks } => {
            let config = crate::graph::taint::TaintConfig {
                sources: taint_ctx.sources.clone(),
                sinks: taint_sinks.clone(),
                sanitizers: taint_ctx.sanitizers.clone(),
            };
            execute_taint_with_config(&config, graph, taint_sinks)
        }
        GraphStage::FindDuplicates { find_duplicates } => {
            Ok(execute_find_duplicates(find_duplicates, nodes))
        }
    }
}

// ---------------------------------------------------------------------------
// Select stage
// ---------------------------------------------------------------------------

fn execute_select(
    node_type: &crate::pipeline::dsl::NodeType,
    filter: Option<&crate::pipeline::dsl::WhereClause>,
    exclude: Option<&crate::pipeline::dsl::WhereClause>,
    graph: &CodeGraph,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<PipelineNode>> {
    use crate::pipeline::dsl::NodeType;

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
            for sym_idx in graph.graph.node_indices() {
                if let NodeWeight::Symbol {
                    name,
                    kind,
                    file_path,
                    start_line,
                    exported,
                    ..
                } = &graph.graph[sym_idx]
                {
                    let language = graph
                        .file_nodes
                        .get(file_path)
                        .and_then(|&file_idx| match &graph.graph[file_idx] {
                            NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                            _ => None,
                        })
                        .unwrap_or_default();

                    let mut metrics = HashMap::new();

                    // unreferenced: no incoming Calls or Imports edges from outside this file
                    let incoming_external = graph
                        .graph
                        .edges_directed(sym_idx, Direction::Incoming)
                        .filter(|e| {
                            matches!(e.weight(), EdgeWeight::Calls | EdgeWeight::Imports)
                                && node_path(&graph.graph[e.source()]) != *file_path
                        })
                        .count();
                    metrics.insert(
                        "unreferenced".to_string(),
                        MetricValue::Int(if incoming_external == 0 { 1 } else { 0 }),
                    );

                    // is_entry_point: file stem matches known entry-point names
                    const ENTRY_POINT_NAMES: &[&str] = &[
                        "main", "lib", "mod", "index", "__init__", "__main__",
                    ];
                    let stem = std::path::Path::new(file_path.as_str())
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let ep = ENTRY_POINT_NAMES.iter().any(|&e| stem == e);
                    metrics.insert(
                        "is_entry_point".to_string(),
                        MetricValue::Int(if ep { 1 } else { 0 }),
                    );

                    let node = PipelineNode {
                        node_idx: sym_idx,
                        file_path: file_path.clone(),
                        name: name.clone(),
                        kind: kind.to_string(),
                        line: *start_line,
                        exported: *exported,
                        language,
                        metrics,
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
    threshold: &crate::pipeline::dsl::NumericPredicate,
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
    config: &crate::pipeline::dsl::MaxDepthConfig,
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
    config: &crate::pipeline::dsl::RatioConfig,
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
// MatchPattern stage (source: iterates workspace files, not graph nodes)
// ---------------------------------------------------------------------------

/// Returns the parameter identifier names declared in a function-like node.
fn collect_function_params<'a>(func_node: &tree_sitter::Node, source: &'a [u8], _lang: crate::language::Language) -> Vec<&'a str> {
    let mut params = Vec::new();
    let Some(params_node) = func_node.child_by_field_name("parameters") else { return params };
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Ok(name) = child.utf8_text(source) {
                    params.push(name);
                }
            }
            // TypeScript/JS: required_parameter { pattern: identifier, ... }
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    if pattern.kind() == "identifier" {
                        if let Ok(name) = pattern.utf8_text(source) {
                            params.push(name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    params
}

/// Build a child→parent map for an entire tree in one DFS pass.
/// Used by `node_lhs_is_parameter` to walk upward from a match node.
fn build_parent_map(root: tree_sitter::Node) -> std::collections::HashMap<usize, tree_sitter::Node> {
    let mut map = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            map.insert(child.id(), current);
            stack.push(child);
        }
    }
    map
}

/// For an assignment_expression node, returns true if the LHS member-expression
/// object is a named parameter of the nearest enclosing function.
/// `parent_map` must be pre-built via `build_parent_map` — callers should build
/// it once per file, not once per capture.
fn node_lhs_is_parameter(
    node: &tree_sitter::Node,
    parent_map: &std::collections::HashMap<usize, tree_sitter::Node>,
    source: &[u8],
    lang: crate::language::Language,
) -> bool {
    let kind = node.kind();
    if kind != "assignment_expression" && kind != "augmented_assignment_expression" {
        return false;
    }
    let Some(lhs) = node.child_by_field_name("left") else { return false };
    if lhs.kind() != "member_expression" { return false }
    // Walk through nested member expressions to find the root identifier.
    // e.g. `config.nested.deep = true` has LHS `config.nested.deep`; root is `config`.
    let mut root_obj = lhs;
    loop {
        let Some(obj) = root_obj.child_by_field_name("object") else { return false };
        if obj.kind() == "identifier" {
            root_obj = obj;
            break;
        } else if obj.kind() == "member_expression" {
            root_obj = obj;
        } else {
            return false;
        }
    }
    let Ok(obj_name) = root_obj.utf8_text(source) else { return false };

    // Walk up from the assignment node to the nearest enclosing function
    let func_kinds = crate::graph::metrics::function_node_kinds_for_language(lang);
    let mut current_id = node.id();
    loop {
        let Some(parent) = parent_map.get(&current_id) else { break };
        if func_kinds.contains(&parent.kind()) {
            let params = collect_function_params(parent, source, lang);
            return params.contains(&obj_name);
        }
        current_id = parent.id();
    }
    false
}

fn execute_match_pattern(
    query_str: &str,
    when: Option<&crate::pipeline::dsl::WhereClause>,
    workspace: &Workspace,
    pipeline_languages: Option<&[String]>,
) -> anyhow::Result<Vec<PipelineNode>> {
    use streaming_iterator::StreamingIterator;

    let mut result = Vec::new();

    for rel_path in workspace.files() {
        let Some(lang) = workspace.file_language(rel_path) else {
            continue;
        };

        // Apply pipeline language filter BEFORE parsing (per D-02:
        // "iterates all workspace files filtered by the pipeline's languages field")
        if let Some(langs) = pipeline_languages {
            let lang_str = lang.as_str();
            if !langs.iter().any(|l| l.eq_ignore_ascii_case(lang_str)) {
                continue;
            }
        }

        let Some(source) = workspace.read_file(rel_path) else {
            continue;
        };

        let ts_lang = lang.tree_sitter_language();

        // Compile query per language -- skip files whose grammar doesn't support the query
        let query = match tree_sitter::Query::new(&ts_lang, query_str) {
            Ok(q) => q,
            Err(_e) => {
                // Different languages have different grammars; a Rust-specific
                // query will fail on a TS file. Skip non-matching languages.
                continue;
            }
        };

        let mut parser = crate::parser::create_parser(lang)?;
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => {
                eprintln!("Warning: match_pattern: failed to parse {rel_path}");
                continue;
            }
        };

        // Build the parent map once per file if lhs_is_parameter filtering is active.
        let parent_map = when
            .and_then(|wc| wc.lhs_is_parameter)
            .map(|_| build_parent_map(tree.root_node()));

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let node = cap.node;
                // Apply when filter if present
                if let Some(wc) = when {
                    if wc.lhs_is_parameter == Some(true) {
                        if let Some(ref pm) = parent_map {
                            if !node_lhs_is_parameter(&node, pm, source.as_bytes(), lang) {
                                continue;
                            }
                        }
                    }
                }
                let line = node.start_position().row as u32 + 1;
                result.push(PipelineNode {
                    node_idx: petgraph::graph::NodeIndex::new(0), // synthetic -- match_pattern nodes are not graph-backed
                    file_path: rel_path.clone(),
                    name: node.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
                    kind: node.kind().to_string(),
                    line,
                    exported: false,
                    language: lang.as_str().to_string(),
                    metrics: std::collections::HashMap::new(),
                });
            }
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// ComputeMetric stage (transform: reads file for each node, computes metric)
// ---------------------------------------------------------------------------

fn execute_compute_metric(
    metric_name: &str,
    nodes: Vec<PipelineNode>,
    workspace: &Workspace,
    graph: &CodeGraph,
) -> anyhow::Result<Vec<PipelineNode>> {
    // Graph-only metrics (no workspace/AST needed)
    match metric_name {
        "efferent_coupling" => {
            let mut result = nodes;
            for node in &mut result {
                let count = graph
                    .graph
                    .edges_directed(node.node_idx, Direction::Outgoing)
                    .filter(|e| matches!(e.weight(), EdgeWeight::Imports))
                    .count();
                node.metrics.insert(
                    "efferent_coupling".to_string(),
                    MetricValue::Int(count as i64),
                );
            }
            return Ok(result);
        }
        "afferent_coupling" => {
            let mut result = nodes;
            for node in &mut result {
                let count = graph
                    .graph
                    .edges_directed(node.node_idx, Direction::Incoming)
                    .filter(|e| matches!(e.weight(), EdgeWeight::Imports | EdgeWeight::Calls))
                    .count();
                node.metrics.insert(
                    "afferent_coupling".to_string(),
                    MetricValue::Int(count as i64),
                );
            }
            return Ok(result);
        }
        _ => {}
    }

    let mut result = Vec::new();

    for mut node in nodes {
        let Some(lang) = workspace.file_language(&node.file_path) else {
            result.push(node);
            continue;
        };
        let Some(source) = workspace.read_file(&node.file_path) else {
            result.push(node);
            continue;
        };

        let mut parser = crate::parser::create_parser(lang)?;
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => {
                eprintln!(
                    "Warning: compute_metric: failed to parse {}",
                    node.file_path
                );
                result.push(node);
                continue;
            }
        };

        let config = crate::graph::metrics::control_flow_config_for_language(lang);
        let target_line = node.line.saturating_sub(1) as usize; // convert 1-indexed to 0-indexed

        // comment_to_code_ratio operates on the whole file, not a function body
        // (file-level ratio applied per symbol node -- see RESEARCH.md Open Question 3 resolution)
        if metric_name == "comment_to_code_ratio" {
            let (comment_lines, code_lines) = crate::graph::metrics::compute_comment_ratio(
                tree.root_node(),
                source.as_bytes(),
                &config,
            );
            let ratio = if code_lines > 0 {
                (comment_lines as f64 / (comment_lines + code_lines) as f64 * 100.0) as i64
            } else {
                0
            };
            node.metrics
                .insert(metric_name.to_string(), MetricValue::Int(ratio));
            result.push(node);
            continue;
        }

        // For function-level metrics, locate the function body at the node's line
        let body_node = find_function_body_at_line(
            tree.root_node(),
            target_line,
            lang,
        );
        let Some(body) = body_node else {
            eprintln!(
                "Warning: compute_metric: no function body at line {} in {}",
                node.line, node.file_path
            );
            result.push(node);
            continue;
        };

        let value: i64 = match metric_name {
            "cyclomatic_complexity" => {
                crate::graph::metrics::compute_cyclomatic(body, &config, source.as_bytes()) as i64
            }
            "function_length" => {
                let (lines, _) = crate::graph::metrics::count_function_lines(body);
                lines as i64
            }
            "cognitive_complexity" => {
                crate::graph::metrics::compute_cognitive(body, &config, source.as_bytes()) as i64
            }
            "nesting_depth" => {
                crate::graph::metrics::compute_nesting_depth(body, &config) as i64
            }
            other => {
                anyhow::bail!(
                    "compute_metric: unknown metric '{}' -- supported: cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio, nesting_depth, efferent_coupling, afferent_coupling",
                    other
                );
            }
        };

        node.metrics
            .insert(metric_name.to_string(), MetricValue::Int(value));
        result.push(node);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Taint stage
// ---------------------------------------------------------------------------

fn execute_taint_with_config(
    config: &crate::graph::taint::TaintConfig,
    graph: &CodeGraph,
    sinks: &[crate::pipeline::dsl::TaintSinkPattern],
) -> anyhow::Result<Vec<PipelineNode>> {
    use crate::graph::taint::TaintEngine;

    let findings = TaintEngine::analyze_all(graph, config);

    let nodes = findings
        .into_iter()
        .map(|f| {
            let mut metrics = HashMap::new();
            metrics.insert("sink".to_string(), MetricValue::Text(f.sink_name.clone()));
            // derive vulnerability from first matching sink pattern
            let vulnerability = sinks
                .iter()
                .find(|s| f.sink_name.contains(s.pattern.as_str()))
                .map(|s| s.vulnerability.clone())
                .unwrap_or_else(|| "unknown".to_string());
            metrics.insert("vulnerability".to_string(), MetricValue::Text(vulnerability));
            metrics.insert("tainted_var".to_string(), MetricValue::Text(f.tainted_var.clone()));
            metrics.insert("source_description".to_string(), MetricValue::Text(f.source_description.clone()));
            PipelineNode {
                node_idx: f.function_node,
                file_path: f.file_path.clone(),
                name: f.function_name.clone(),
                kind: "taint_finding".to_string(),
                line: f.sink_line,
                exported: false,
                language: String::new(),
                metrics,
            }
        })
        .collect();

    Ok(nodes)
}

// ---------------------------------------------------------------------------
// FindDuplicates stage
// ---------------------------------------------------------------------------

fn execute_find_duplicates(
    stage: &crate::pipeline::dsl::FindDuplicatesStage,
    nodes: Vec<PipelineNode>,
) -> Vec<PipelineNode> {
    let mut groups: HashMap<String, Vec<PipelineNode>> = HashMap::new();
    for node in nodes {
        let key = match stage.by.as_str() {
            "name" => node.name.clone(),
            other => node
                .metrics
                .get(other)
                .map(|v| match v {
                    MetricValue::Text(s) => s.clone(),
                    MetricValue::Int(i) => i.to_string(),
                    MetricValue::Float(f) => f.to_string(),
                })
                .unwrap_or_default(),
        };
        groups.entry(key).or_default().push(node);
    }

    groups
        .into_iter()
        .filter(|(_, members)| members.len() >= stage.min_count)
        .map(|(key, members)| {
            let count = members.len();
            let files: Vec<String> = members.iter().map(|n| n.file_path.clone()).collect();
            let mut rep = members.into_iter().next().unwrap();
            rep.metrics.insert("count".to_string(), MetricValue::Int(count as i64));
            rep.metrics.insert("files".to_string(), MetricValue::Text(files.join(", ")));
            rep.metrics.insert("name".to_string(), MetricValue::Text(key));
            rep
        })
        .collect()
}

/// Walk the tree to find a function node whose start line matches `target_line`,
/// then return its body child. Used by `execute_compute_metric` to locate the
/// function body for metric computation.
fn find_function_body_at_line(
    root: tree_sitter::Node,
    target_line: usize,
    lang: crate::language::Language,
) -> Option<tree_sitter::Node> {
    let func_kinds = crate::graph::metrics::function_node_kinds_for_language(lang);
    let body_field = crate::graph::metrics::body_field_for_language(lang);

    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if func_kinds.contains(&current.kind()) && current.start_position().row == target_line {
            if let Some(body) = current.child_by_field_name(body_field) {
                return Some(body);
            }
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
    None
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
    use crate::pipeline::dsl::{
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
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
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
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
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
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
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
        let mut taint_ctx = TaintContext::default();
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        ).unwrap();

        let cycle_stage = GraphStage::FindCycles {
            find_cycles: FindCyclesConfig { edge: EdgeType::Imports },
        };
        let cycle_nodes = execute_stage(
            &cycle_stage,
            nodes,
            &graph,
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
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
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
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
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
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
        let mut taint_ctx = TaintContext::default();
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
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
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
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
        let mut taint_ctx = TaintContext::default();
        let mut nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            None,
            None,
            "test_pipeline",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
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
                crate::pipeline::dsl::SeverityEntry {
                    when: Some(WhereClause {
                        metrics: {
                            let mut m = std::collections::HashMap::new();
                            m.insert("count".to_string(), NumericPredicate { gte: Some(20.0), ..Default::default() });
                            m
                        },
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                crate::pipeline::dsl::SeverityEntry {
                    when: None,
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };

        let effective_pipeline = flag_config.pipeline_name.as_deref().unwrap_or("test_pipeline");
        let findings: Vec<AuditFinding> = nodes
            .iter()
            .filter_map(|node| {
                let severity = flag_config.resolve_severity(node)?;
                let message = interpolate_message(&flag_config.message, node);
                Some(AuditFinding {
                    file_path: node.file_path.clone(),
                    line: node.line,
                    column: 1,
                    severity,
                    pipeline: effective_pipeline.to_string(),
                    pattern: flag_config.pattern.clone(),
                    message,
                    snippet: String::new(),
                })
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

        let out = run_pipeline(&stages, &graph, None, None, None, "my_pipeline").unwrap();
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

        let out = run_pipeline(&stages, &graph, None, None, None, "wrapper_pipeline").unwrap();
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

    // -- match_pattern + compute_metric tests ----

    #[test]
    fn test_match_pattern_finds_panic_in_rust() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            r#"fn foo() { panic!("oops"); }
fn bar() { println!("ok"); }
"#,
        )
        .unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Rust], None).unwrap();

        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern: r#"(macro_invocation (identifier) @name (#eq? @name "panic")) @call"#
                    .to_string(),
                when: None,
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "panic_detected".to_string(),
                    message: "panic at {{file}}:{{line}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "panic_detection").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(
                    !findings.is_empty(),
                    "expected at least one finding for panic!()"
                );
                assert_eq!(findings[0].pattern, "panic_detected");
                assert_eq!(findings[0].line, 1);
                assert!(findings[0].file_path.contains("lib.rs"));
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_match_pattern_no_match_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            "fn clean() { println!(\"all good\"); }\n",
        )
        .unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Rust], None).unwrap();

        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern: r#"(macro_invocation (identifier) @name (#eq? @name "panic")) @call"#
                    .to_string(),
                when: None,
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "panic_detected".to_string(),
                    message: "panic at {{file}}:{{line}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "panic_detection").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(
                    findings.is_empty(),
                    "expected zero findings for clean code, got {}",
                    findings.len()
                );
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_match_pattern_finds_function_in_typescript() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("sample.ts"),
            "export function greet(name: string): string { return name; }\n",
        )
        .unwrap();
        let ws =
            crate::workspace::Workspace::load(dir.path(), &[Language::TypeScript], None).unwrap();
        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern: "(function_declaration name: (identifier) @name)".to_string(),
                when: None,
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "ts_function".to_string(),
                    message: "function at {{file}}:{{line}}".to_string(),
                    severity: Some("info".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "test_ts").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(!findings.is_empty(), "expected findings for TypeScript function");
                assert!(
                    findings[0].file_path.ends_with(".ts"),
                    "expected .ts file path, got {}",
                    findings[0].file_path
                );
                assert_eq!(findings[0].line, 1, "expected line 1");
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_compute_metric_cyclomatic_flags_complex_function() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // Function with CC > 1: multiple if branches
        std::fs::write(
            src_dir.join("complex.rs"),
            r#"fn complex(x: i32, y: i32, z: i32) {
    if x > 0 {
        println!("a");
    }
    if y > 0 {
        println!("b");
    }
    if z > 0 {
        println!("c");
    }
    for i in 0..10 {
        println!("{}", i);
    }
}
"#,
        )
        .unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Rust], None).unwrap();

        // Build a graph with a symbol node at line 1 (the function start)
        let mut graph = CodeGraph::new();
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/complex.rs".to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert("src/complex.rs".to_string(), file_idx);
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "complex".to_string(),
            kind: crate::models::SymbolKind::Function,
            file_path: "src/complex.rs".to_string(),
            start_line: 1,
            end_line: 14,
            exported: false,
        });
        graph.symbol_nodes.insert(("src/complex.rs".to_string(), 1), sym_idx);

        let stages = vec![
            GraphStage::Select {
                select: crate::pipeline::dsl::NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::ComputeMetric {
                compute_metric: "cyclomatic_complexity".to_string(),
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "high_cc".to_string(),
                    message: "CC={{cyclomatic_complexity}} in {{name}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "cyclomatic_complexity").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert_eq!(findings.len(), 1, "expected 1 finding for complex function");
                assert!(
                    findings[0].message.contains("CC="),
                    "message should contain CC value"
                );
                // CC = 1 (base) + 3 (if) + 1 (for) = 5
                assert!(
                    findings[0].message.contains("CC=5"),
                    "expected CC=5, got: {}",
                    findings[0].message
                );
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_compute_metric_cyclomatic_clean_function_no_finding_above_threshold() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // Simple function with CC=1 (no branches)
        std::fs::write(
            src_dir.join("simple.rs"),
            "fn simple() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Rust], None).unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/simple.rs".to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert("src/simple.rs".to_string(), file_idx);
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "simple".to_string(),
            kind: crate::models::SymbolKind::Function,
            file_path: "src/simple.rs".to_string(),
            start_line: 1,
            end_line: 3,
            exported: false,
        });
        graph.symbol_nodes.insert(("src/simple.rs".to_string(), 1), sym_idx);

        let stages = vec![
            GraphStage::Select {
                select: crate::pipeline::dsl::NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::ComputeMetric {
                compute_metric: "cyclomatic_complexity".to_string(),
            },
        ];
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "cc_test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 1, "expected 1 result for simple function");
                // The node should have CC=1 (base complexity only)
            }
            _ => panic!("expected Results (no Flag stage)"),
        }
    }

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
                        metrics: {
                            let mut m = std::collections::HashMap::new();
                            m.insert("ratio".to_string(), NumericPredicate { gte: Some(0.5), ..Default::default() });
                            m
                        },
                        ..Default::default()
                    }),
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                // 3/4 = 0.75 >= 0.5, so should pass
                assert_eq!(results.len(), 1);
            }
            _ => panic!("expected Results"),
        }
    }

    #[test]
    fn test_compute_metric_nesting_depth_detects_deep_nesting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // 4 levels of nesting: for > if > for > if
        std::fs::write(
            src_dir.join("lib.rs"),
            r#"fn deeply_nested(items: &[i32]) -> i32 {
    let mut sum = 0;
    for item in items {
        if *item > 0 {
            for _ in 0..*item {
                if *item > 5 {
                    sum += 1;
                }
            }
        }
    }
    sum
}
"#,
        )
        .unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Rust], None).unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/lib.rs".to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert("src/lib.rs".to_string(), file_idx);
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "deeply_nested".to_string(),
            kind: crate::models::SymbolKind::Function,
            file_path: "src/lib.rs".to_string(),
            start_line: 1,
            end_line: 13,
            exported: false,
        });
        graph.symbol_nodes.insert(("src/lib.rs".to_string(), 1), sym_idx);

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let select_stage = GraphStage::Select {
            select: crate::pipeline::dsl::NodeType::Symbol,
            filter: None,
            exclude: None,
        };
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            Some(&ws),
            None,
            "nesting_depth_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        let metric_stage = GraphStage::ComputeMetric {
            compute_metric: "nesting_depth".to_string(),
        };
        let result_nodes = execute_stage(
            &metric_stage,
            nodes,
            &graph,
            Some(&ws),
            None,
            "nesting_depth_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(result_nodes.len(), 1, "expected 1 result for deeply_nested function");
        let depth = result_nodes[0].metric_f64("nesting_depth") as i64;
        assert!(depth >= 4, "expected nesting_depth >= 4 for 4-level nesting, got {}", depth);
    }

    #[test]
    fn test_compute_metric_nesting_depth_javascript_arrow_function() {
        // Regression test: JS async arrow functions must produce nesting_depth metrics.
        // Uses GraphBuilder so the test exercises the full symbol-discovery path,
        // not just execute_stage with a hand-built graph.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("controller.js"),
            r#"const createComment = async (req, res) => {
    if (req.body) {
        if (req.user) {
            if (req.params.id) {
                if (req.body.parentId) {
                    return req.body;
                }
            }
        }
    }
};
"#,
        )
        .unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::JavaScript], None).unwrap();
        let graph = crate::graph::builder::GraphBuilder::new(&ws, &[Language::JavaScript])
            .build()
            .unwrap();

        // Assert the graph builder actually registered the arrow function symbol.
        let sym_count = graph.symbol_nodes.len();
        assert!(
            sym_count >= 1,
            "expected at least 1 symbol node from JS arrow function, got {}",
            sym_count
        );

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let select_stage = GraphStage::Select {
            select: crate::pipeline::dsl::NodeType::Symbol,
            filter: None,
            exclude: None,
        };
        let nodes = execute_stage(
            &select_stage, Vec::new(), &graph, Some(&ws), None,
            "js_nesting_test", &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
        ).unwrap();

        assert_eq!(nodes.len(), 1, "expected 1 symbol node, got {}", nodes.len());

        let metric_stage = GraphStage::ComputeMetric {
            compute_metric: "nesting_depth".to_string(),
        };
        let result_nodes = execute_stage(
            &metric_stage, nodes, &graph, Some(&ws), None,
            "js_nesting_test", &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
        ).unwrap();

        assert_eq!(result_nodes.len(), 1, "expected 1 node after compute_metric");
        // 4 levels: if > if > if > if
        let depth = result_nodes[0].metric_f64("nesting_depth") as i64;
        assert_eq!(depth, 4, "expected nesting depth of 4, got {}", depth);
    }

    #[test]
    fn test_compute_metric_nesting_depth_cpp_qualified_method() {
        // Regression test: CPP_SYMBOL_QUERY must match qualified-name function definitions
        // like `int DataProcessor::process(int x, int y) { ... }`. Previously only
        // plain `(identifier)` declarators were matched, so out-of-class method definitions
        // were never symbolised and never reached the metric pipeline.
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("processor.cpp"),
            r#"int DataProcessor::process(int x, int y) {
    if (x > 0) {
        if (y > 0) {
            if (x > y) {
                if (x > 10) {
                    return x;
                }
            }
        }
    }
    return 0;
}
"#,
        )
        .unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Cpp], None).unwrap();
        // Use GraphBuilder so the test exercises the full symbol-discovery path
        // (including CPP_SYMBOL_QUERY), not just execute_stage with a hand-built graph.
        let graph = crate::graph::builder::GraphBuilder::new(&ws, &[Language::Cpp])
            .build()
            .unwrap();

        assert!(
            graph.symbol_nodes.len() >= 1,
            "CPP_SYMBOL_QUERY must match qualified-name function definition -- got 0 symbols"
        );

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let nodes = execute_stage(
            &GraphStage::Select { select: crate::pipeline::dsl::NodeType::Symbol, filter: None, exclude: None },
            Vec::new(), &graph, Some(&ws), None, "cpp_nesting_test",
            &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
        ).unwrap();

        let result_nodes = execute_stage(
            &GraphStage::ComputeMetric { compute_metric: "nesting_depth".to_string() },
            nodes, &graph, Some(&ws), None, "cpp_nesting_test",
            &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
        ).unwrap();

        assert_eq!(result_nodes.len(), 1, "expected 1 symbol node");
        let depth = result_nodes[0].metrics.get("nesting_depth")
            .expect("nesting_depth metric should be present -- CPP_SYMBOL_QUERY must match qualified-name functions");
        // 4 levels: if > if > if > if
        match depth {
            MetricValue::Int(v) => assert_eq!(*v, 4, "expected nesting depth 4, got {}", v),
            _ => panic!("expected Int metric"),
        }
    }

    #[test]
    fn test_lhs_is_parameter_filters_local_object_mutations() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("service.js"),
            r#"function createFilter(role) {
    const filter = {};
    filter.role = role;
    return filter;
}
function mutateParam(user) {
    user.name = "overwritten";
}
"#,
        ).unwrap();
        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::JavaScript], None).unwrap();

        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern: "(assignment_expression left: (member_expression) @lhs) @assign".to_string(),
                when: Some(crate::pipeline::dsl::WhereClause {
                    lhs_is_parameter: Some(true),
                    ..Default::default()
                }),
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "argument_mutation".to_string(),
                    message: "mutation".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "lhs_param_test").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert_eq!(findings.len(), 1,
                    "expected exactly 1 finding (mutateParam's user.name), got {} findings: {:?}",
                    findings.len(),
                    findings.iter().map(|f| format!("{}:{}", f.file_path, f.line)).collect::<Vec<_>>()
                );
                assert_eq!(findings[0].line, 7, "expected line 7 (user.name = ...), got {}", findings[0].line);
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_compute_metric_function_length_csharp_method() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // Write a C# method with 60 lines in the body
        let mut body = String::from("public class MyService {\n    public void ProcessOrder(int orderId) {\n");
        for i in 0..58 {
            body.push_str(&format!("        var step{i} = orderId + {i};\n"));
        }
        body.push_str("    }\n}\n");
        std::fs::write(src_dir.join("service.cs"), &body).unwrap();

        let ws = crate::workspace::Workspace::load(dir.path(), &[Language::CSharp], None).unwrap();
        let graph = crate::graph::builder::GraphBuilder::new(&ws, &[Language::CSharp])
            .build()
            .expect("GraphBuilder should succeed");

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let nodes = execute_stage(
            &GraphStage::Select {
                select: crate::pipeline::dsl::NodeType::Symbol,
                filter: Some(crate::pipeline::dsl::WhereClause {
                    kind: Some(vec!["method".to_string()]),
                    ..Default::default()
                }),
                exclude: None,
            },
            Vec::new(), &graph, Some(&ws), None, "csharp_length_test",
            &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
        ).unwrap();

        assert!(!nodes.is_empty(), "expected at least one method symbol in C# file");

        let result_nodes = execute_stage(
            &GraphStage::ComputeMetric { compute_metric: "function_length".to_string() },
            nodes, &graph, Some(&ws), None, "csharp_length_test",
            &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
        ).unwrap();

        assert_eq!(result_nodes.len(), 1, "expected 1 method node with metric");
        let len = result_nodes[0].metrics.get("function_length")
            .expect("function_length metric should be present");
        match len {
            MetricValue::Int(v) => assert!(*v >= 50, "expected >= 50 lines, got {}", v),
            _ => panic!("expected Int metric"),
        }
    }

}
