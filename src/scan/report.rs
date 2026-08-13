//! Findings produced by the review agents.
//!
//! ponytail: data types only — rendering lives in a later task.

use anyhow::Result;
use cersei::prelude::schemars; // cersei re-exports schemars 0.8; do NOT `cargo add schemars`
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How bad a finding is. Declaration order is the sort order, so an
/// ascending sort puts `High` first.
///
/// `JsonSchema` is required because `ReportFindingInput` embeds this type
/// and cersei's `ToolExecute::Input` bound is
/// `DeserializeOwned + schemars::JsonSchema`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Which review reported it (filled by the runner, not the agent).
    #[serde(default)]
    pub review: String,
    pub severity: Severity,
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}

/// ponytail: stub. It prints JSON and ignores `json` / `output`; the
/// next task replaces the body with real text + JSON rendering. The
/// signature is already the final one, so the runner's call site stands.
pub fn emit(
    findings: &[Finding],
    failures: &[String],
    _json: bool,
    _output: Option<&Path>,
) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "findings": findings,
            "failures": failures,
        }))?
    );
    Ok(())
}
