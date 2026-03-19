use anyhow::Result;
use serde::Serialize;

use crate::cli::OutputFormat;

use super::models::{AuditFinding, AuditSummary};

/// Format a vector of serializable rows into the requested output format.
fn format_output<T: Serialize>(
    rows: &[T],
    headers: &[&str],
    format: &OutputFormat,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(rows)?),
        OutputFormat::Csv => {
            let mut out = String::new();
            out.push_str(&headers.join(","));
            out.push('\n');
            for row in rows {
                let value = serde_json::to_value(row)?;
                let cols: Vec<String> = headers
                    .iter()
                    .map(|h| {
                        value
                            .get(h)
                            .map(|v| match v {
                                serde_json::Value::String(s) => {
                                    if s.contains(',') || s.contains('"') {
                                        format!("\"{}\"", s.replace('"', "\"\""))
                                    } else {
                                        s.clone()
                                    }
                                }
                                other => other.to_string(),
                            })
                            .unwrap_or_default()
                    })
                    .collect();
                out.push_str(&cols.join(","));
                out.push('\n');
            }
            Ok(out)
        }
        OutputFormat::Table => {
            if rows.is_empty() {
                return Ok("(no results)\n".to_string());
            }
            let json_rows: Vec<serde_json::Value> = rows
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?;
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for row in &json_rows {
                for (i, header) in headers.iter().enumerate() {
                    let cell = match row.get(header) {
                        None | Some(serde_json::Value::Null) => 0,
                        Some(serde_json::Value::String(s)) => s.len(),
                        Some(v) => v.to_string().len(),
                    };
                    if cell > widths[i] {
                        widths[i] = cell;
                    }
                }
            }
            let mut out = String::new();
            let header_cells: Vec<String> = headers
                .iter()
                .enumerate()
                .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
                .collect();
            out.push_str(&header_cells.join("  "));
            out.push('\n');
            let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            out.push_str(&sep.join("  "));
            out.push('\n');
            for row in &json_rows {
                let cells: Vec<String> = headers
                    .iter()
                    .enumerate()
                    .map(|(i, h)| {
                        let cell = match row.get(h) {
                            None | Some(serde_json::Value::Null) => String::new(),
                            Some(serde_json::Value::String(s)) => s.clone(),
                            Some(serde_json::Value::Bool(b)) => b.to_string(),
                            Some(v) => v.to_string(),
                        };
                        format!("{:<width$}", cell, width = widths[i])
                    })
                    .collect();
                out.push_str(&cells.join("  "));
                out.push('\n');
            }
            Ok(out)
        }
    }
}

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
                    .map(|(_, pats)| pats.iter().map(|(p, c)| format!("{p}: {c}")).collect())
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
                "file_path",
                "line",
                "column",
                "severity",
                "pipeline",
                "pattern",
                "message",
                "snippet",
            ];
            format_output(display_findings, headers, format)
        }
    }
}

#[derive(Serialize)]
struct SummaryJson {
    files_scanned: usize,
    files_with_findings: usize,
    total_findings: usize,
    categories: Vec<CategoryJson>,
}

#[derive(Serialize)]
struct CategoryJson {
    name: String,
    findings: usize,
    pipelines: Vec<PipelineJson>,
}

#[derive(Serialize)]
struct PipelineJson {
    name: String,
    count: usize,
    patterns: Vec<PatternJson>,
}

#[derive(Serialize)]
struct PatternJson {
    name: String,
    count: usize,
}

pub fn format_code_quality_summary(
    summaries: &[(&str, &AuditSummary)],
    format: &OutputFormat,
    title: Option<&str>,
) -> Result<String> {
    let title = title.unwrap_or("Code Quality Report");
    let total_findings: usize = summaries.iter().map(|(_, s)| s.total_findings).sum();
    let files_scanned: usize = summaries.iter().map(|(_, s)| s.files_scanned).sum();
    let files_with_findings: usize = summaries.iter().map(|(_, s)| s.files_with_findings).sum();

    match format {
        OutputFormat::Table => {
            let mut out = String::new();
            out.push_str(&format!("{title}\n"));
            out.push_str(&"=".repeat(title.len()));
            out.push('\n');
            out.push_str(&format!("Files scanned:        {files_scanned}\n"));
            out.push_str(&format!("Files with findings:  {files_with_findings}\n"));
            out.push_str(&format!("Total findings:       {total_findings}\n\n"));

            for (name, summary) in summaries {
                out.push_str(&format!("--- {name} ---\n"));
                for (pipeline_name, count) in &summary.by_pipeline {
                    let patterns: Vec<String> = summary
                        .by_pipeline_pattern
                        .iter()
                        .find(|(n, _)| n == pipeline_name)
                        .map(|(_, pats)| pats.iter().map(|(p, c)| format!("{p}: {c}")).collect())
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "  {pipeline_name}: {count} ({})\n",
                        patterns.join(", ")
                    ));
                }
                out.push('\n');
            }

            Ok(out)
        }
        OutputFormat::Json => {
            let summary = SummaryJson {
                files_scanned,
                files_with_findings,
                total_findings,
                categories: summaries
                    .iter()
                    .map(|(name, s)| CategoryJson {
                        name: name.to_string(),
                        findings: s.total_findings,
                        pipelines: s
                            .by_pipeline_pattern
                            .iter()
                            .map(|(pipeline, patterns)| PipelineJson {
                                name: pipeline.clone(),
                                count: s
                                    .by_pipeline
                                    .iter()
                                    .find(|(n, _)| n == pipeline)
                                    .map(|(_, c)| *c)
                                    .unwrap_or(0),
                                patterns: patterns
                                    .iter()
                                    .map(|(p, c)| PatternJson {
                                        name: p.clone(),
                                        count: *c,
                                    })
                                    .collect(),
                            })
                            .collect(),
                    })
                    .collect(),
            };
            Ok(serde_json::to_string_pretty(&summary)?)
        }
        OutputFormat::Csv => {
            let mut out = String::new();
            out.push_str("category,pipeline,pattern,count\n");
            for (name, summary) in summaries {
                for (pipeline, patterns) in &summary.by_pipeline_pattern {
                    for (pattern, count) in patterns {
                        out.push_str(&format!("{name},{pipeline},{pattern},{count}\n"));
                    }
                }
            }
            Ok(out)
        }
    }
}
