pub struct AuditSummary {
    pub total_findings: usize,
    pub files_scanned: usize,
    pub files_with_findings: usize,
    pub by_pipeline: Vec<(String, usize)>,
    pub by_pattern: Vec<(String, usize)>,
    pub by_pipeline_pattern: Vec<(String, Vec<(String, usize)>)>,
}
