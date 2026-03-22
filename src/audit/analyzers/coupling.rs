use crate::audit::models::AuditFinding;
use crate::audit::project_analyzer::ProjectAnalyzer;
use crate::audit::project_index::ProjectIndex;

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

    fn analyze(&self, index: &ProjectIndex) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let reverse = index.reverse_edges();

        // Efferent coupling (fan-out): how many files does this file depend on?
        for (node, targets) in &index.edges {
            if targets.len() >= EFFERENT_THRESHOLD {
                findings.push(AuditFinding {
                    file_path: node.path().to_string(),
                    line: 1,
                    column: 1,
                    severity: "warning".to_string(),
                    pipeline: "cross_file_coupling".to_string(),
                    pattern: "high_efferent_coupling".to_string(),
                    message: format!(
                        "High efferent coupling: {} depends on {} other files/packages (threshold: {})",
                        node.path(),
                        targets.len(),
                        EFFERENT_THRESHOLD
                    ),
                    snippet: String::new(),
                });
            }
        }

        // Afferent coupling (fan-in): how many files depend on this file?
        for (node, sources) in &reverse {
            if sources.len() >= AFFERENT_THRESHOLD {
                findings.push(AuditFinding {
                    file_path: node.path().to_string(),
                    line: 1,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: "cross_file_coupling".to_string(),
                    pattern: "high_afferent_coupling".to_string(),
                    message: format!(
                        "Hub module: {} files/packages depend on {} (threshold: {})",
                        sources.len(),
                        node.path(),
                        AFFERENT_THRESHOLD
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
    use crate::audit::project_index::GraphNode;
    use std::collections::HashSet;

    #[test]
    fn detects_high_efferent() {
        let mut index = ProjectIndex::new();
        let hub = GraphNode::File("hub.rs".into());
        let targets: HashSet<GraphNode> = (0..12)
            .map(|i| GraphNode::File(format!("dep{}.rs", i)))
            .collect();
        index.edges.insert(hub, targets);

        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&index);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "high_efferent_coupling");
    }

    #[test]
    fn detects_high_afferent() {
        let mut index = ProjectIndex::new();
        let target = GraphNode::File("core.rs".into());
        for i in 0..16 {
            let source = GraphNode::File(format!("user{}.rs", i));
            index
                .edges
                .entry(source)
                .or_default()
                .insert(target.clone());
        }

        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&index);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "high_afferent_coupling");
    }

    #[test]
    fn no_findings_below_threshold() {
        let mut index = ProjectIndex::new();
        let a = GraphNode::File("a.rs".into());
        let b = GraphNode::File("b.rs".into());
        index.edges.insert(a, HashSet::from([b]));

        let analyzer = CouplingAnalyzer;
        let findings = analyzer.analyze(&index);
        assert!(findings.is_empty());
    }
}
