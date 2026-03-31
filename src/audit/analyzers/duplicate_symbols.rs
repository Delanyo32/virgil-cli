use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::graph::{CodeGraph, NodeWeight};
use crate::models::SymbolKind;

pub struct DuplicateSymbolsAnalyzer;

impl ProjectAnalyzer for DuplicateSymbolsAnalyzer {
    fn name(&self) -> &str {
        "cross_file_duplicates"
    }

    fn description(&self) -> &str {
        "Detect exported symbols with identical names across files"
    }

    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        for (name, indices) in &graph.symbols_by_name {
            // Collect exported symbols with their file+kind info
            let exported: Vec<(&str, SymbolKind, u32)> = indices
                .iter()
                .filter_map(|&idx| match &graph.graph[idx] {
                    NodeWeight::Symbol {
                        file_path,
                        kind,
                        start_line,
                        exported,
                        ..
                    } => {
                        if *exported {
                            Some((file_path.as_str(), *kind, *start_line))
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
                .collect();

            if exported.len() < 2 {
                continue;
            }

            // Group by kind — only flag duplicates of the same kind
            let mut by_kind: std::collections::HashMap<SymbolKind, Vec<(&str, u32)>> =
                std::collections::HashMap::new();
            for (file_path, kind, line) in &exported {
                by_kind.entry(*kind).or_default().push((file_path, *line));
            }

            for (kind, locations) in &by_kind {
                if locations.len() < 2 {
                    continue;
                }

                let other_files: Vec<String> =
                    locations.iter().map(|(p, _)| p.to_string()).collect();
                let message = format!(
                    "Cross-file duplicate: {} '{}' exported from {} files: {}",
                    kind,
                    name,
                    other_files.len(),
                    other_files.join(", ")
                );

                for (file_path, line) in locations {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: *line,
                        column: 1,
                        severity: "info".to_string(),
                        pipeline: "cross_file_duplicates".to_string(),
                        pattern: "cross_file_duplicate".to_string(),
                        message: message.clone(),
                        snippet: String::new(),
                    });
                }
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

    #[test]
    fn detects_cross_file_duplicates() {
        let mut graph = CodeGraph::new();

        for path in &["src/a.rs", "src/b.rs"] {
            let file_idx = graph.graph.add_node(NodeWeight::File {
                path: path.to_string(),
                language: Language::Rust,
            });
            graph.file_nodes.insert(path.to_string(), file_idx);

            let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
                name: "parse_config".to_string(),
                kind: SymbolKind::Function,
                file_path: path.to_string(),
                start_line: 1,
                end_line: 10,
                exported: true,
            });
            graph
                .symbols_by_name
                .entry("parse_config".to_string())
                .or_default()
                .push(sym_idx);
        }

        let analyzer = DuplicateSymbolsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.pattern == "cross_file_duplicate"));
    }

    #[test]
    fn no_duplicate_when_not_exported() {
        let mut graph = CodeGraph::new();

        for path in &["src/a.rs", "src/b.rs"] {
            let file_idx = graph.graph.add_node(NodeWeight::File {
                path: path.to_string(),
                language: Language::Rust,
            });
            graph.file_nodes.insert(path.to_string(), file_idx);

            let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
                name: "helper".to_string(),
                kind: SymbolKind::Function,
                file_path: path.to_string(),
                start_line: 1,
                end_line: 10,
                exported: false,
            });
            graph
                .symbols_by_name
                .entry("helper".to_string())
                .or_default()
                .push(sym_idx);
        }

        let analyzer = DuplicateSymbolsAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }
}
