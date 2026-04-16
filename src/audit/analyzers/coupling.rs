use std::collections::HashSet;

use petgraph::Direction;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::audit::models::AuditFinding;
use crate::audit::pipelines::helpers::{is_barrel_file, is_excluded_for_arch_analysis};
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::graph::{CodeGraph, EdgeWeight};

/// Minimum efferent threshold regardless of project statistics.
const EFFERENT_FLOOR: usize = 8;
/// Minimum afferent threshold regardless of project statistics.
const AFFERENT_FLOOR: usize = 10;

pub struct CouplingAnalyzer;

impl ProjectAnalyzer for CouplingAnalyzer {
    fn name(&self) -> &str {
        "cross_file_coupling"
    }

    fn description(&self) -> &str {
        "Detect structural high-fan-out (efferent) and high-fan-in (afferent) coupling \
         from the import graph. Operates at the project level — complements per-file \
         excessive_imports checks in per-language pipelines."
    }

    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding> {
        // Build the set of internal (non-excluded) file nodes once for O(1) lookup.
        let internal_nodes: HashSet<NodeIndex> = graph
            .file_nodes
            .iter()
            .filter(|(path, _)| !is_excluded_for_arch_analysis(path))
            .map(|(_, &idx)| idx)
            .collect();

        // First pass: collect coupling counts to compute adaptive thresholds.
        // Efferent counts come from non-barrel internal files only.
        // Afferent counts come from all internal files.
        let mut efferent_counts: Vec<(NodeIndex, String, usize)> = Vec::new();
        let mut afferent_counts: Vec<(NodeIndex, String, usize)> = Vec::new();

        for (path, &file_idx) in &graph.file_nodes {
            if !internal_nodes.contains(&file_idx) {
                continue;
            }

            // Afferent: incoming Imports edges from internal files
            let afferent = graph
                .graph
                .edges_directed(file_idx, Direction::Incoming)
                .filter(|e| {
                    matches!(e.weight(), EdgeWeight::Imports)
                        && internal_nodes.contains(&e.source())
                })
                .count();
            afferent_counts.push((file_idx, path.clone(), afferent));

            // Efferent: outgoing Imports edges to internal files (skip barrels)
            if !is_barrel_file(path) {
                let efferent = graph
                    .graph
                    .edges_directed(file_idx, Direction::Outgoing)
                    .filter(|e| {
                        matches!(e.weight(), EdgeWeight::Imports)
                            && internal_nodes.contains(&e.target())
                    })
                    .count();
                efferent_counts.push((file_idx, path.clone(), efferent));
            }
        }

        let efferent_threshold = compute_adaptive_threshold(
            &efferent_counts.iter().map(|(_, _, c)| *c).collect::<Vec<_>>(),
            EFFERENT_FLOOR,
        );
        let afferent_threshold = compute_adaptive_threshold(
            &afferent_counts.iter().map(|(_, _, c)| *c).collect::<Vec<_>>(),
            AFFERENT_FLOOR,
        );

        // Second pass: emit findings for files above adaptive thresholds.
        let mut findings = Vec::new();

        for (_, path, efferent) in &efferent_counts {
            if *efferent >= efferent_threshold {
                findings.push(AuditFinding {
                    file_path: path.clone(),
                    line: 1,
                    column: 1,
                    severity: coupling_severity(*efferent, efferent_threshold),
                    pipeline: "cross_file_coupling".to_string(),
                    pattern: "high_efferent_coupling".to_string(),
                    message: format!(
                        "High efferent coupling: {} depends on {} internal files (threshold: {})",
                        path, efferent, efferent_threshold
                    ),
                    snippet: String::new(),
                });
            }
        }

        for (_, path, afferent) in &afferent_counts {
            if *afferent >= afferent_threshold {
                findings.push(AuditFinding {
                    file_path: path.clone(),
                    line: 1,
                    column: 1,
                    severity: coupling_severity(*afferent, afferent_threshold),
                    pipeline: "cross_file_coupling".to_string(),
                    pattern: "high_afferent_coupling".to_string(),
                    message: format!(
                        "Hub module: {} internal files depend on {} (threshold: {})",
                        afferent, path, afferent_threshold
                    ),
                    snippet: String::new(),
                });
            }
        }

        findings
    }
}

/// Compute a project-relative threshold as `ceil(mean + 2 * stddev)`,
/// floored at `min_floor` to prevent false negatives on small/uniform projects.
fn compute_adaptive_threshold(values: &[usize], min_floor: usize) -> usize {
    if values.is_empty() {
        return min_floor;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<usize>() as f64 / n;
    let variance = values
        .iter()
        .map(|&v| {
            let d = v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    let threshold = (mean + 2.0 * variance.sqrt()).ceil() as usize;
    threshold.max(min_floor)
}

/// Graduate severity by how far the count exceeds the threshold:
/// - < 1.5x → "info"
/// - 1.5x–2x → "warning"
/// - ≥ 2x → "error"
fn coupling_severity(count: usize, threshold: usize) -> String {
    if count >= threshold * 2 {
        "error".to_string()
    } else if count * 2 >= threshold * 3 {
        // count >= threshold * 1.5
        "warning".to_string()
    } else {
        "info".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeWeight, NodeWeight};
    use crate::language::Language;

    fn add_file(graph: &mut CodeGraph, path: &str) -> NodeIndex {
        let idx = graph.graph.add_node(NodeWeight::File {
            path: path.to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert(path.to_string(), idx);
        idx
    }

    fn add_import(graph: &mut CodeGraph, from: NodeIndex, to: NodeIndex) {
        graph.graph.add_edge(from, to, EdgeWeight::Imports);
    }

    #[test]
    fn detects_high_efferent() {
        let mut graph = CodeGraph::new();
        let hub = add_file(&mut graph, "hub.rs");
        for i in 0..12 {
            let dep = add_file(&mut graph, &format!("dep{}.rs", i));
            add_import(&mut graph, hub, dep);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.iter().any(|f| f.pattern == "high_efferent_coupling"));
    }

    #[test]
    fn detects_high_afferent() {
        let mut graph = CodeGraph::new();
        let core = add_file(&mut graph, "core.rs");
        for i in 0..16 {
            let user = add_file(&mut graph, &format!("user{}.rs", i));
            add_import(&mut graph, user, core);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.iter().any(|f| f.pattern == "high_afferent_coupling"));
    }

    #[test]
    fn no_findings_below_threshold() {
        let mut graph = CodeGraph::new();
        let a = add_file(&mut graph, "a.rs");
        let b = add_file(&mut graph, "b.rs");
        add_import(&mut graph, a, b);
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_test_file_excluded_from_efferent() {
        // A test file with many imports should not appear in findings
        let mut graph = CodeGraph::new();
        let test_hub = add_file(&mut graph, "integration_test.rs");
        for i in 0..20 {
            let dep = add_file(&mut graph, &format!("svc{}.rs", i));
            add_import(&mut graph, test_hub, dep);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(
            !findings.iter().any(|f| f.file_path == "integration_test.rs"),
            "Test files should be excluded"
        );
    }

    #[test]
    fn test_vendor_file_excluded() {
        let mut graph = CodeGraph::new();
        let vendor = add_file(&mut graph, "vendor/lib/core.rs");
        for i in 0..20 {
            let dep = add_file(&mut graph, &format!("dep{}.rs", i));
            add_import(&mut graph, vendor, dep);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(
            !findings.iter().any(|f| f.file_path == "vendor/lib/core.rs"),
            "Vendor files should be excluded"
        );
    }

    #[test]
    fn test_barrel_file_not_flagged_for_efferent() {
        let mut graph = CodeGraph::new();
        let barrel = add_file(&mut graph, "src/index.ts");
        for i in 0..12 {
            let dep = add_file(&mut graph, &format!("mod{}.ts", i));
            add_import(&mut graph, barrel, dep);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(
            !findings
                .iter()
                .any(|f| f.file_path == "src/index.ts" && f.pattern == "high_efferent_coupling"),
            "Barrel files should not be flagged for efferent coupling"
        );
    }

    #[test]
    fn test_barrel_file_still_flagged_for_afferent() {
        // A barrel that is imported by many files should still be flagged for afferent
        let mut graph = CodeGraph::new();
        let barrel = add_file(&mut graph, "src/index.ts");
        for i in 0..20 {
            let user = add_file(&mut graph, &format!("page{}.ts", i));
            add_import(&mut graph, user, barrel);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(
            findings
                .iter()
                .any(|f| f.file_path == "src/index.ts" && f.pattern == "high_afferent_coupling"),
            "Barrel files should still be flagged for afferent coupling"
        );
    }

    #[test]
    fn test_external_import_not_counted() {
        // An edge from hub.rs to a node NOT in file_nodes should not be counted
        let mut graph = CodeGraph::new();
        let hub = add_file(&mut graph, "hub.rs");
        // Add an external symbol node (not in file_nodes) and edge to it
        let ext = graph.graph.add_node(NodeWeight::File {
            path: "tokio".to_string(),
            language: Language::Rust,
        });
        // Intentionally do NOT add "tokio" to graph.file_nodes
        graph.graph.add_edge(hub, ext, EdgeWeight::Imports);
        // Also add 3 real internal deps
        for i in 0..3 {
            let dep = add_file(&mut graph, &format!("real{}.rs", i));
            add_import(&mut graph, hub, dep);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        // With only 3 internal imports, should not trigger efferent (floor is 8)
        assert!(
            !findings.iter().any(|f| f.pattern == "high_efferent_coupling"),
            "External imports must not be counted toward efferent coupling"
        );
    }

    #[test]
    fn test_adaptive_threshold_small_project() {
        // 3 files each with 1 import — no findings expected (floor protects)
        let mut graph = CodeGraph::new();
        let a = add_file(&mut graph, "a.rs");
        let b = add_file(&mut graph, "b.rs");
        let c = add_file(&mut graph, "c.rs");
        add_import(&mut graph, a, b);
        add_import(&mut graph, b, c);
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_severity_graduation_efferent() {
        // Build a project where one file has efferent count well above threshold.
        // 20 background files with 1 import each (mean ~1, stddev ~0, threshold = floor=8)
        // Outlier file with 20 imports → 20 >= 8*2 → "error"
        let mut graph = CodeGraph::new();
        // Background files
        for i in 0..20 {
            let a = add_file(&mut graph, &format!("bg_a{}.rs", i));
            let b = add_file(&mut graph, &format!("bg_b{}.rs", i));
            add_import(&mut graph, a, b);
        }
        // Outlier with 20 outgoing imports
        let outlier = add_file(&mut graph, "outlier.rs");
        for i in 0..20 {
            let dep = add_file(&mut graph, &format!("odep{}.rs", i));
            add_import(&mut graph, outlier, dep);
        }
        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&graph);
        let ef = findings
            .iter()
            .find(|f| f.file_path == "outlier.rs" && f.pattern == "high_efferent_coupling");
        assert!(ef.is_some(), "Outlier should be flagged");
        assert_eq!(ef.unwrap().severity, "error");
    }
}
