use petgraph::Direction;

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::graph::{CodeGraph, EdgeWeight};

const EFFERENT_THRESHOLD: usize = 10;
const AFFERENT_THRESHOLD: usize = 15;

pub struct CouplingAnalyzer;

impl ProjectAnalyzer for CouplingAnalyzer {
    fn name(&self) -> &str {
        "cross_file_coupling"
    }

    fn description(&self) -> &str {
        "Detect high efferent (fan-out) and afferent (fan-in) coupling"
    }

    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        for (&ref path, &file_idx) in &graph.file_nodes {
            // Efferent: outgoing Imports edges
            let efferent = graph
                .graph
                .edges_directed(file_idx, Direction::Outgoing)
                .filter(|e| matches!(e.weight(), EdgeWeight::Imports))
                .count();

            if efferent >= EFFERENT_THRESHOLD {
                findings.push(AuditFinding {
                    file_path: path.clone(),
                    line: 1,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: "cross_file_coupling".to_string(),
                    pattern: "high_efferent_coupling".to_string(),
                    message: format!(
                        "High efferent coupling: {} depends on {} other files/packages (threshold: {})",
                        path, efferent, EFFERENT_THRESHOLD
                    ),
                    snippet: String::new(),
                });
            }

            // Afferent: incoming Imports edges
            let afferent = graph
                .graph
                .edges_directed(file_idx, Direction::Incoming)
                .filter(|e| matches!(e.weight(), EdgeWeight::Imports))
                .count();

            if afferent >= AFFERENT_THRESHOLD {
                findings.push(AuditFinding {
                    file_path: path.clone(),
                    line: 1,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "cross_file_coupling".to_string(),
                    pattern: "high_afferent_coupling".to_string(),
                    message: format!(
                        "Hub module: {} files/packages depend on {} (threshold: {})",
                        afferent, path, AFFERENT_THRESHOLD
                    ),
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
    use std::collections::HashMap;

    fn add_file(graph: &mut CodeGraph, path: &str) {
        let idx = graph.graph.add_node(NodeWeight::File {
            path: path.to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert(path.to_string(), idx);
    }

    #[test]
    fn detects_high_efferent() {
        let mut graph = CodeGraph::new();
        add_file(&mut graph, "hub.rs");
        for i in 0..12 {
            let dep = format!("dep{}.rs", i);
            add_file(&mut graph, &dep);
            let from = graph.file_nodes["hub.rs"];
            let to = graph.file_nodes[&dep];
            graph.graph.add_edge(from, to, EdgeWeight::Imports);
        }

        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.iter().any(|f| f.pattern == "high_efferent_coupling"));
    }

    #[test]
    fn detects_high_afferent() {
        let mut graph = CodeGraph::new();
        add_file(&mut graph, "core.rs");
        for i in 0..16 {
            let user = format!("user{}.rs", i);
            add_file(&mut graph, &user);
            let from = graph.file_nodes[&user];
            let to = graph.file_nodes["core.rs"];
            graph.graph.add_edge(from, to, EdgeWeight::Imports);
        }

        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.iter().any(|f| f.pattern == "high_afferent_coupling"));
    }

    #[test]
    fn no_findings_below_threshold() {
        let mut graph = CodeGraph::new();
        add_file(&mut graph, "a.rs");
        add_file(&mut graph, "b.rs");
        let a = graph.file_nodes["a.rs"];
        let b = graph.file_nodes["b.rs"];
        graph.graph.add_edge(a, b, EdgeWeight::Imports);

        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }
}
