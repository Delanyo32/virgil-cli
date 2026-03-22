use std::collections::{HashMap, HashSet};

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::audit::project_index::{GraphNode, ProjectIndex};

pub struct CircularDepsAnalyzer;

impl ProjectAnalyzer for CircularDepsAnalyzer {
    fn name(&self) -> &str {
        "circular_dependencies"
    }

    fn description(&self) -> &str {
        "Detect circular dependency cycles using Tarjan's SCC algorithm"
    }

    fn analyze(&self, index: &ProjectIndex) -> Vec<AuditFinding> {
        let sccs = tarjan_scc(&index.edges);
        let mut findings = Vec::new();

        for scc in sccs {
            if scc.len() < 2 {
                continue;
            }

            let participants: Vec<String> = scc.iter().map(|n| n.path().to_string()).collect();
            let message = format!(
                "Circular dependency cycle involving {} files/packages: {}",
                participants.len(),
                participants.join(" -> ")
            );

            // Report one finding per file in the cycle
            for node in &scc {
                let file_path = node.path().to_string();
                findings.push(AuditFinding {
                    file_path,
                    line: 1,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: "circular_dependencies".to_string(),
                    pattern: "circular_dependency".to_string(),
                    message: message.clone(),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

/// Tarjan's strongly connected components algorithm.
fn tarjan_scc(edges: &HashMap<GraphNode, HashSet<GraphNode>>) -> Vec<Vec<GraphNode>> {
    // Collect all nodes
    let mut all_nodes: HashSet<&GraphNode> = HashSet::new();
    for (from, tos) in edges {
        all_nodes.insert(from);
        for to in tos {
            all_nodes.insert(to);
        }
    }

    let mut state = TarjanState {
        index_counter: 0,
        stack: Vec::new(),
        on_stack: HashSet::new(),
        indices: HashMap::new(),
        lowlinks: HashMap::new(),
        sccs: Vec::new(),
    };

    for node in &all_nodes {
        if !state.indices.contains_key(*node) {
            strongconnect(node, edges, &mut state);
        }
    }

    state.sccs
}

struct TarjanState<'a> {
    index_counter: usize,
    stack: Vec<&'a GraphNode>,
    on_stack: HashSet<&'a GraphNode>,
    indices: HashMap<&'a GraphNode, usize>,
    lowlinks: HashMap<&'a GraphNode, usize>,
    sccs: Vec<Vec<GraphNode>>,
}

fn strongconnect<'a>(
    node: &'a GraphNode,
    edges: &'a HashMap<GraphNode, HashSet<GraphNode>>,
    state: &mut TarjanState<'a>,
) {
    state.indices.insert(node, state.index_counter);
    state.lowlinks.insert(node, state.index_counter);
    state.index_counter += 1;
    state.stack.push(node);
    state.on_stack.insert(node);

    if let Some(neighbors) = edges.get(node) {
        for neighbor in neighbors {
            if !state.indices.contains_key(neighbor) {
                strongconnect(neighbor, edges, state);
                let neighbor_lowlink = state.lowlinks[neighbor];
                let node_lowlink = state.lowlinks.get_mut(node).unwrap();
                if neighbor_lowlink < *node_lowlink {
                    *node_lowlink = neighbor_lowlink;
                }
            } else if state.on_stack.contains(neighbor) {
                let neighbor_index = state.indices[neighbor];
                let node_lowlink = state.lowlinks.get_mut(node).unwrap();
                if neighbor_index < *node_lowlink {
                    *node_lowlink = neighbor_index;
                }
            }
        }
    }

    if state.lowlinks[node] == state.indices[node] {
        let mut scc = Vec::new();
        loop {
            let w = state.stack.pop().unwrap();
            state.on_stack.remove(w);
            scc.push(w.clone());
            if w == node {
                break;
            }
        }
        state.sccs.push(scc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_simple_cycle() {
        let mut index = ProjectIndex::new();
        let a = GraphNode::File("a.rs".into());
        let b = GraphNode::File("b.rs".into());

        let mut edges = HashMap::new();
        edges.insert(a.clone(), HashSet::from([b.clone()]));
        edges.insert(b.clone(), HashSet::from([a.clone()]));
        index.edges = edges;

        let analyzer = CircularDepsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert!(!findings.is_empty());
        assert!(findings.iter().all(|f| f.pattern == "circular_dependency"));
    }

    #[test]
    fn no_cycle_in_dag() {
        let mut index = ProjectIndex::new();
        let a = GraphNode::File("a.rs".into());
        let b = GraphNode::File("b.rs".into());
        let c = GraphNode::File("c.rs".into());

        let mut edges = HashMap::new();
        edges.insert(a.clone(), HashSet::from([b.clone()]));
        edges.insert(b.clone(), HashSet::from([c.clone()]));
        index.edges = edges;

        let analyzer = CircularDepsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_three_node_cycle() {
        let mut index = ProjectIndex::new();
        let a = GraphNode::File("a.rs".into());
        let b = GraphNode::File("b.rs".into());
        let c = GraphNode::File("c.rs".into());

        let mut edges = HashMap::new();
        edges.insert(a.clone(), HashSet::from([b.clone()]));
        edges.insert(b.clone(), HashSet::from([c.clone()]));
        edges.insert(c.clone(), HashSet::from([a.clone()]));
        index.edges = edges;

        let analyzer = CircularDepsAnalyzer;
        let findings = analyzer.analyze(&index);
        assert_eq!(findings.len(), 3); // One per file in cycle
    }
}
