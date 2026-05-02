use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::Direction;
use petgraph::algo::tarjan_scc;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, NodeWeight};
use crate::pipeline::dsl::{EdgeType, MaxDepthConfig, MetricValue, PipelineNode};
use crate::pipeline::helpers::is_barrel_file;
use crate::pipeline::node_helpers::{edge_matches_type, node_path, pipeline_node_from_index};

pub(crate) fn execute_find_cycles(
    edge_type: &EdgeType,
    nodes: Vec<PipelineNode>,
    graph: &CodeGraph,
) -> anyhow::Result<Vec<PipelineNode>> {
    let node_set: HashSet<NodeIndex> = nodes.iter().map(|n| n.node_idx).collect();
    let edge_weight_matches = |ew: &crate::graph::EdgeWeight| edge_matches_type(ew, edge_type);

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
        if let (Some(&sub_src), Some(&sub_tgt)) = (orig_to_sub.get(&src), orig_to_sub.get(&tgt)) {
            import_graph.add_edge(sub_src, sub_tgt, ());
        }
    }

    let sccs = tarjan_scc(&import_graph);
    let mut result = Vec::new();

    for scc in sccs {
        if scc.len() < 2 {
            continue;
        }

        let orig_indices: Vec<NodeIndex> = scc.iter().map(|&sub| import_graph[sub]).collect();

        let participants: Vec<String> = orig_indices
            .iter()
            .filter_map(|&idx| match &graph.graph[idx] {
                NodeWeight::File { path, .. } => Some(path.clone()),
                NodeWeight::Symbol { file_path, .. } => Some(file_path.clone()),
                _ => None,
            })
            .collect();

        let cycle_size = scc.len();
        let scc_set: HashSet<NodeIndex> = orig_indices.iter().copied().collect();
        let cycle_path = ordered_cycle_path_for_edge(&orig_indices, &scc_set, graph, edge_type);

        let representative_path = participants.iter().min().cloned().unwrap_or_default();
        let rep_node_idx = orig_indices
            .iter()
            .find(|&&idx| {
                let path = node_path(&graph.graph[idx]);
                path == representative_path
            })
            .copied()
            .unwrap_or(orig_indices[0]);

        let rep_base =
            pipeline_node_from_index(rep_node_idx, graph).unwrap_or_else(|| PipelineNode {
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
        rep.metrics.insert(
            "cycle_size".to_string(),
            MetricValue::Int(cycle_size as i64),
        );
        rep.metrics
            .insert("cycle_path".to_string(), MetricValue::Text(cycle_path));

        result.push(rep);
    }

    Ok(result)
}

pub(crate) fn execute_max_depth(
    config: &MaxDepthConfig,
    nodes: Vec<PipelineNode>,
    graph: &CodeGraph,
) -> anyhow::Result<Vec<PipelineNode>> {
    let skip_barrels = config.skip_barrel_files.unwrap_or(false);
    let edge_type = &config.edge;

    let node_set: HashSet<NodeIndex> = nodes.iter().map(|n| n.node_idx).collect();
    let node_by_idx: HashMap<NodeIndex, &PipelineNode> =
        nodes.iter().map(|n| (n.node_idx, n)).collect();

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

    let roots: VecDeque<NodeIndex> = in_degree
        .iter()
        .filter(|&(_, deg)| *deg == 0)
        .map(|(&idx, _)| idx)
        .collect();

    let mut depth_map: HashMap<NodeIndex, usize> =
        node_set.iter().map(|&idx| (idx, 0)).collect();

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

    for &idx in &node_set {
        if !visited_kahn.contains(&idx) {
            topo_order.push(idx);
        }
    }

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
