use std::collections::{HashMap, VecDeque};

use petgraph::Direction;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};

const DEEP_CHAIN_THRESHOLD: usize = 6;

pub struct DependencyDepthAnalyzer;

impl ProjectAnalyzer for DependencyDepthAnalyzer {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detect deep dependency chains in the import graph"
    }

    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding> {
        let depths = compute_depths(graph);
        let mut findings = Vec::new();

        for (idx, depth) in &depths {
            if *depth >= DEEP_CHAIN_THRESHOLD {
                let path = match &graph.graph[*idx] {
                    NodeWeight::File { path, .. } => path.clone(),
                    _ => continue,
                };
                findings.push(AuditFinding {
                    file_path: path.clone(),
                    line: 1,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "dependency_graph_depth".to_string(),
                    pattern: "deep_dependency_chain".to_string(),
                    message: format!(
                        "Deep dependency chain: {} is at depth {} in the import graph (threshold: {})",
                        path, depth, DEEP_CHAIN_THRESHOLD
                    ),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

fn compute_depths(graph: &CodeGraph) -> HashMap<NodeIndex, usize> {
    // Find root file nodes (no incoming Imports edges)
    let mut has_incoming: std::collections::HashSet<NodeIndex> = std::collections::HashSet::new();
    for edge in graph.graph.edge_references() {
        if matches!(edge.weight(), EdgeWeight::Imports) {
            has_incoming.insert(edge.target());
        }
    }

    let mut depths: HashMap<NodeIndex, usize> = HashMap::new();
    let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();

    for &file_idx in graph.file_nodes.values() {
        if !has_incoming.contains(&file_idx) {
            depths.insert(file_idx, 0);
            queue.push_back((file_idx, 0));
        }
    }

    while let Some((node, depth)) = queue.pop_front() {
        for edge in graph.graph.edges_directed(node, Direction::Outgoing) {
            if matches!(edge.weight(), EdgeWeight::Imports) {
                let target = edge.target();
                let new_depth = depth + 1;
                let current = depths.get(&target).copied().unwrap_or(0);
                if new_depth > current {
                    depths.insert(target, new_depth);
                    queue.push_back((target, new_depth));
                }
            }
        }
    }

    depths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeWeight, NodeWeight};
    use crate::language::Language;

    fn make_chain(n: usize) -> CodeGraph {
        let mut graph = CodeGraph::new();
        let mut prev = None;
        for i in 0..n {
            let path = format!("f{}.rs", i);
            let idx = graph.graph.add_node(NodeWeight::File {
                path: path.clone(),
                language: Language::Rust,
            });
            graph.file_nodes.insert(path, idx);
            if let Some(prev_idx) = prev {
                graph.graph.add_edge(prev_idx, idx, EdgeWeight::Imports);
            }
            prev = Some(idx);
        }
        graph
    }

    #[test]
    fn shallow_graph_no_findings() {
        let graph = make_chain(2);
        let analyzer = DependencyDepthAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }

    #[test]
    fn deep_chain_detected() {
        let graph = make_chain(8);
        let analyzer = DependencyDepthAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(!findings.is_empty());
        assert!(
            findings
                .iter()
                .all(|f| f.pattern == "deep_dependency_chain")
        );
    }
}
