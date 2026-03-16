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
    page: usize,
    per_page: usize,
) -> Result<String> {
    let total = findings.len();
    let page = if page == 0 { 1 } else { page };
    let start = (page - 1) * per_page;
    let end = std::cmp::min(start + per_page, total);

    let display_findings = if start >= total {
        &findings[0..0]
    } else {
        &findings[start..end]
    };

    match format {
        OutputFormat::Table => {
            let mut output = String::new();

            for finding in display_findings {
                output.push_str(&format!(
                    "[{}] {}/{} -- {}:{}:{}\n",
                    finding.severity,
                    finding.pipeline,
                    finding.pattern,
                    finding.file_path,
                    finding.line,
                    finding.column,
                ));
                output.push_str(&format!("  {}\n", finding.message));
                if !finding.snippet.is_empty() {
                    for line in finding.snippet.lines() {
                        output.push_str(&format!("  > {line}\n"));
                    }
                }
                output.push('\n');
            }

            let total_pages = if total == 0 {
                1
            } else {
                (total + per_page - 1) / per_page
            };
            let shown_start = if total == 0 { 0 } else { start + 1 };
            let shown_end = end;
            output.push_str(&format!(
                "Page {page} of {total_pages} (showing {shown_start}-{shown_end} of {total} findings)\n"
            ));

            output.push_str(&format!(
                "{} findings in {} files ({} files scanned)\n",
                summary.total_findings, summary.files_with_findings, summary.files_scanned
            ));

            for (pipeline_name, count) in &summary.by_pipeline {
                let patterns: Vec<String> = summary
                    .by_pipeline_pattern
                    .iter()
                    .find(|(name, _)| name == pipeline_name)
                    .map(|(_, pats)| {
                        pats.iter()
                            .map(|(p, c)| format!("{p}: {c}"))
                            .collect()
                    })
                    .unwrap_or_default();
                output.push_str(&format!(
                    "  {pipeline_name}: {count} ({})\n",
                    patterns.join(", ")
                ));
            }

            Ok(output)
        }
        _ => {
            let headers = &[
                "file_path", "line", "column", "severity", "pipeline", "pattern", "message",
                "snippet",
            ];
            format_output(display_findings, headers, format)
        }
    }
}
