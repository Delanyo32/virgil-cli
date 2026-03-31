use crate::graph::CodeGraph;

use super::models::AuditFinding;

/// Cross-file analyzer that operates on the whole-project graph.
/// Separate from Pipeline (which is per-file, per-tree-sitter parse).
pub trait ProjectAnalyzer: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding>;
}
