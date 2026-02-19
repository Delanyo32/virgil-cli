use anyhow::Result;
use serde::Serialize;

use crate::cli::OutputFormat;

/// Format a vector of serializable rows into the requested output format.
pub fn format_output<T: Serialize>(rows: &[T], headers: &[&str], format: &OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => {
            Ok(serde_json::to_string_pretty(rows)?)
        }
        OutputFormat::Csv => {
            format_csv(rows, headers)
        }
        OutputFormat::Table => {
            format_table(rows, headers)
        }
    }
}

fn format_csv<T: Serialize>(rows: &[T], headers: &[&str]) -> Result<String> {
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

fn format_table<T: Serialize>(rows: &[T], headers: &[&str]) -> Result<String> {
    if rows.is_empty() {
        return Ok("(no results)\n".to_string());
    }

    // Convert all rows to JSON values to extract fields
    let json_rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| serde_json::to_value(r))
        .collect::<Result<Vec<_>, _>>()?;

    // Calculate column widths
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &json_rows {
        for (i, header) in headers.iter().enumerate() {
            let cell = cell_display(row.get(header));
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    let mut out = String::new();

    // Header row
    let header_cells: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
        .collect();
    out.push_str(&header_cells.join("  "));
    out.push('\n');

    // Separator
    let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    out.push_str(&sep.join("  "));
    out.push('\n');

    // Data rows
    for row in &json_rows {
        let cells: Vec<String> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let cell = cell_display(row.get(h));
                format!("{:<width$}", cell, width = widths[i])
            })
            .collect();
        out.push_str(&cells.join("  "));
        out.push('\n');
    }

    Ok(out)
}

fn cell_display(value: Option<&serde_json::Value>) -> String {
    match value {
        None => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(v) => v.to_string(),
    }
}

/// Format a single labeled section for overview output.
pub fn format_section(title: &str, content: &str) -> String {
    format!("=== {} ===\n{}\n", title, content)
}
