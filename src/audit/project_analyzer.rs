use super::models::AuditFinding;
use super::project_index::ProjectIndex;

/// Cross-file analyzer that operates on the whole-project index.
/// Separate from Pipeline (which is per-file, per-tree-sitter parse).
pub trait ProjectAnalyzer: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn analyze(&self, index: &ProjectIndex) -> Vec<AuditFinding>;
}
