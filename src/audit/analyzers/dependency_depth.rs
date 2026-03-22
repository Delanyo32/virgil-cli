use std::collections::{HashMap, HashSet, VecDeque};

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::audit::project_index::{GraphNode, ProjectIndex};

const DEEP_CHAIN_THRESHOLD: usize = 6;

pub struct DependencyDepthAnalyzer;

impl ProjectAnalyzer for DependencyDepthAnalyzer {
    fn name(&self) -> &str {
        "dependency_graph_depth"
    }

    fn description(&self) -> &str {
        "Detect deep dependency chains in the import graph"
    }

    fn analyze(&self, index: &ProjectIndex) -> Vec<AuditFinding> {
        let depths = compute_depths(&index.edges);
        let mut findings = Vec::new();

        for (node, depth) in &depths {
            if *depth >= DEEP_CHAIN_THRESHOLD {
                findings.push(AuditFinding {
                    file_path: node.path().to_string(),
                    line: 1,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "dependency_graph_depth".to_string(),
                    pattern: "deep_dependency_chain".to_string(),
                    message: format!(
                        "Deep dependency chain: {} is at depth {} in the import graph (threshold: {})",
                        node.path(),
                        depth,
                        DEEP_CHAIN_THRESHOLD
                    ),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

/// BFS from root nodes (no incoming edges) to compute depth of each node.
fn compute_depths(edges: &HashMap<GraphNode, HashSet<GraphNode>>) -> HashMap<GraphNode, usize> {
    // Collect all nodes
    let mut all_nodes: HashSet<GraphNode> = HashSet::new();
    let mut has_incoming: HashSet<GraphNode> = HashSet::new();
    for (from, tos) in edges {
        all_nodes.insert(from.clone());
        for to in tos {
            all_nodes.insert(to.clone());
            has_incoming.insert(to.clone());
        }
    }

    // Root nodes = no incoming edges
    let roots: Vec<GraphNode> = all_nodes
        .iter()
        .filter(|n| !has_incoming.contains(*n))
        .cloned()
        .collect();

    let mut depths: HashMap<GraphNode, usize> = HashMap::new();
    let mut queue: VecDeque<(GraphNode, usize)> = VecDeque::new();

    for root in roots {
        depths.insert(root.clone(), 0);
        queue.push_back((root, 0));
    }

    // BFS — take maximum depth for each node
    while let Some((node, depth)) = queue.pop_front() {
        if let Some(neighbors) = edges.get(&node) {
            for neighbor in neighbors {
                let new_depth = depth + 1;
                let current = depths.get(neighbor).copied().unwrap_or(0);
                if new_depth > current {
                    depths.insert(neighbor.clone(), new_depth);
                    queue.push_back((neighbor.clone(), new_depth));
                }
            }
        }
    }

    depths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shallow_graph_no_findings() {
        let mut index = ProjectIndex::new();
        let a = GraphNode::File("a.rs".into());
        let b = GraphNode::File("b.rs".into());

        let mut edges = HashMap::new();
        edges.insert(a, HashSet::from([b]));
        index.edges = edges;

        let analyzer = DependencyDepthAnalyzer;
        let findings = analyzer.analyze(&index);
        assert!(findings.is_empty());
    }

    #[test]
    fn deep_chain_detected() {
        let mut index = ProjectIndex::new();
        let nodes: Vec<GraphNode> = (0..8)
            .map(|i| GraphNode::File(format!("f{}.rs", i)))
            .collect();

        let mut edges = HashMap::new();
        for i in 0..7 {
            edges.insert(nodes[i].clone(), HashSet::from([nodes[i + 1].clone()]));
        }
        index.edges = edges;

        let analyzer = DependencyDepthAnalyzer;
        let findings = analyzer.analyze(&index);
        // Nodes at depth >= 6 should be flagged
        assert!(!findings.is_empty());
        assert!(
            findings
                .iter()
                .all(|f| f.pattern == "deep_dependency_chain")
        );
    }
}
