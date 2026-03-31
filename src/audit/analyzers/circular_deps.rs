use petgraph::algo::tarjan_scc;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};

pub struct CircularDepsAnalyzer;

impl ProjectAnalyzer for CircularDepsAnalyzer {
    fn name(&self) -> &str {
        "circular_dependencies"
    }

    fn description(&self) -> &str {
        "Detect circular dependency cycles using Tarjan's SCC algorithm"
    }

    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding> {
        // Build a subgraph of only File nodes connected by Imports edges
        let mut import_graph = petgraph::graph::DiGraph::<NodeIndex, ()>::new();
        let mut idx_map: std::collections::HashMap<NodeIndex, NodeIndex> =
            std::collections::HashMap::new();

        // Add file nodes
        for &file_idx in graph.file_nodes.values() {
            let sub_idx = import_graph.add_node(file_idx);
            idx_map.insert(file_idx, sub_idx);
        }

        // Add import edges
        for edge in graph.graph.edge_references() {
            if matches!(edge.weight(), EdgeWeight::Imports)
                && let (Some(&from_sub), Some(&to_sub)) =
                    (idx_map.get(&edge.source()), idx_map.get(&edge.target()))
            {
                import_graph.add_edge(from_sub, to_sub, ());
            }
        }

        // Run Tarjan's SCC
        let sccs = tarjan_scc(&import_graph);
        let mut findings = Vec::new();

        for scc in sccs {
            if scc.len() < 2 {
                continue;
            }

            // Map back to original file paths
            let participants: Vec<String> = scc
                .iter()
                .filter_map(|&sub_idx| {
                    let orig_idx = import_graph[sub_idx];
                    match &graph.graph[orig_idx] {
                        NodeWeight::File { path, .. } => Some(path.clone()),
                        _ => None,
                    }
                })
                .collect();

            let message = format!(
                "Circular dependency cycle involving {} files/packages: {}",
                participants.len(),
                participants.join(" -> ")
            );

            for file_path in &participants {
                findings.push(AuditFinding {
                    file_path: file_path.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeWeight, NodeWeight};
    use crate::language::Language;

    fn make_graph_with_imports(edges: &[(&str, &str)]) -> CodeGraph {
        let mut graph = CodeGraph::new();

        // Collect unique file paths
        let mut paths: Vec<&str> = edges.iter().flat_map(|(a, b)| [*a, *b]).collect();
        paths.sort();
        paths.dedup();

        for path in &paths {
            let idx = graph.graph.add_node(NodeWeight::File {
                path: path.to_string(),
                language: Language::Rust,
            });
            graph.file_nodes.insert(path.to_string(), idx);
        }

        for (from, to) in edges {
            let from_idx = graph.file_nodes[*from];
            let to_idx = graph.file_nodes[*to];
            graph.graph.add_edge(from_idx, to_idx, EdgeWeight::Imports);
        }

        graph
    }

    #[test]
    fn detects_simple_cycle() {
        let graph = make_graph_with_imports(&[("a.rs", "b.rs"), ("b.rs", "a.rs")]);
        let analyzer = CircularDepsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(!findings.is_empty());
        assert!(findings.iter().all(|f| f.pattern == "circular_dependency"));
    }

    #[test]
    fn no_cycle_in_dag() {
        let graph = make_graph_with_imports(&[("a.rs", "b.rs"), ("b.rs", "c.rs")]);
        let analyzer = CircularDepsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_three_node_cycle() {
        let graph =
            make_graph_with_imports(&[("a.rs", "b.rs"), ("b.rs", "c.rs"), ("c.rs", "a.rs")]);
        let analyzer = CircularDepsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert_eq!(findings.len(), 3);
    }
}
