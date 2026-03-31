use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};

/// Entry-point file names that should not be flagged as dead exports.
const ENTRY_POINT_NAMES: &[&str] = &["main", "lib", "mod", "index", "__init__", "__main__"];

pub struct DeadExportsAnalyzer;

impl ProjectAnalyzer for DeadExportsAnalyzer {
    fn name(&self) -> &str {
        "dead_exports"
    }

    fn description(&self) -> &str {
        "Detect exported symbols that are never referenced by any other file"
    }

    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        for sym_idx in graph.graph.node_indices() {
            let (name, kind, file_path, start_line, exported) = match &graph.graph[sym_idx] {
                NodeWeight::Symbol {
                    name,
                    kind,
                    file_path,
                    start_line,
                    exported,
                    ..
                } => (name, kind, file_path, *start_line, *exported),
                _ => continue,
            };

            if !exported {
                continue;
            }

            // Skip entry-point files
            if is_entry_point(file_path) {
                continue;
            }

            // Skip main functions
            if name == "main" || name == "__init__" {
                continue;
            }

            // Check if any incoming Calls edge comes from a symbol in a different file
            let has_cross_file_caller = graph
                .graph
                .edges_directed(sym_idx, Direction::Incoming)
                .any(|edge| {
                    if !matches!(edge.weight(), EdgeWeight::Calls) {
                        return false;
                    }
                    // Check if caller is in a different file
                    match &graph.graph[edge.source()] {
                        NodeWeight::Symbol {
                            file_path: caller_file,
                            ..
                        } => caller_file != file_path,
                        _ => false,
                    }
                });

            if !has_cross_file_caller {
                findings.push(AuditFinding {
                    file_path: file_path.clone(),
                    line: start_line,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "dead_exports".to_string(),
                    pattern: "dead_export".to_string(),
                    message: format!(
                        "Exported {} '{}' is not referenced by any other file in the project",
                        kind, name
                    ),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

fn is_entry_point(path: &str) -> bool {
    let file_stem = path
        .rsplit_once('/')
        .map(|(_, f)| f)
        .unwrap_or(path)
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(path);

    ENTRY_POINT_NAMES.contains(&file_stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeWeight, NodeWeight};
    use crate::language::Language;
    use crate::models::SymbolKind;

    #[test]
    fn detects_dead_export() {
        let mut graph = CodeGraph::new();

        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/utils.rs".to_string(),
            language: Language::Rust,
        });
        graph
            .file_nodes
            .insert("src/utils.rs".to_string(), file_idx);

        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "format_date".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/utils.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });
        graph.graph.add_edge(sym_idx, file_idx, EdgeWeight::DefinedIn);

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "dead_export");
        assert!(findings[0].message.contains("format_date"));
    }

    #[test]
    fn no_finding_when_called_cross_file() {
        let mut graph = CodeGraph::new();

        let file_a = graph.graph.add_node(NodeWeight::File {
            path: "src/utils.rs".to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert("src/utils.rs".to_string(), file_a);

        let sym_a = graph.graph.add_node(NodeWeight::Symbol {
            name: "format_date".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/utils.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });

        let file_b = graph.graph.add_node(NodeWeight::File {
            path: "src/handler.rs".to_string(),
            language: Language::Rust,
        });
        graph
            .file_nodes
            .insert("src/handler.rs".to_string(), file_b);

        let sym_b = graph.graph.add_node(NodeWeight::Symbol {
            name: "handle".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/handler.rs".to_string(),
            start_line: 1,
            end_line: 10,
            exported: false,
        });

        // sym_b calls sym_a
        graph.graph.add_edge(sym_b, sym_a, EdgeWeight::Calls);

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_entry_points() {
        let mut graph = CodeGraph::new();

        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/main.rs".to_string(),
            language: Language::Rust,
        });
        graph
            .file_nodes
            .insert("src/main.rs".to_string(), file_idx);

        let _sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "run".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/main.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });

        let analyzer = DeadExportsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }
}
