pub mod engine;
pub mod models;
pub mod pipeline;
pub mod pipelines;
pub mod primitives;

use anyhow::Result;

use crate::cli::OutputFormat;
use crate::query::format::format_output;

use self::models::{AuditFinding, AuditSummary};

pub fn format_findings(
    findings: &[AuditFinding],
    summary: &AuditSummary,
    format: &OutputFormat,
    limit: usize,
) -> Result<String> {
    let display_findings = if findings.len() > limit {
        &findings[..limit]
    } else {
        findings
    };

    let headers = &[
        "file_path", "line", "column", "severity", "pipeline", "pattern", "message", "snippet",
    ];

    let mut output = format_output(display_findings, headers, format)?;

    if matches!(format, OutputFormat::Table) {
        output.push('\n');

        if findings.len() > limit {
            output.push_str(&format!(
                "Showing {} of {} findings (use --limit to see more)\n",
                limit, summary.total_findings
            ));
        }

        output.push_str(&format!(
            "{} findings in {} files ({} files scanned)\n",
            summary.total_findings, summary.files_with_findings, summary.files_scanned
        ));

        for (pipeline_name, count) in &summary.by_pipeline {
            let patterns: Vec<String> = summary
                .by_pattern
                .iter()
                .map(|(p, c)| format!("{p}: {c}"))
                .collect();
            output.push_str(&format!(
                "  {pipeline_name}: {count} ({})\n",
                patterns.join(", ")
            ));
        }
    }

    Ok(output)
}
