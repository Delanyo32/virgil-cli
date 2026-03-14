use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AuditFinding {
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub severity: String,
    pub pipeline: String,
    pub pattern: String,
    pub message: String,
    pub snippet: String,
}

pub struct AuditSummary {
    pub total_findings: usize,
    pub files_scanned: usize,
    pub files_with_findings: usize,
    pub by_pipeline: Vec<(String, usize)>,
    pub by_pattern: Vec<(String, usize)>,
}
