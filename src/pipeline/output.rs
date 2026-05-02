//! Pipeline output types — what `run_pipeline` produces.
//!
//! `AuditFinding` is emitted by pipelines ending in a Flag stage; `QueryResult`
//! is the per-symbol result emitted by every other pipeline shape and by the
//! query engine directly.

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

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub end_line: u32,
    pub column: u32,
    pub exported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}
