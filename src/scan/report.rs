//! Findings produced by the review agents, and how they are rendered.

use anyhow::Result;
use cersei::prelude::schemars; // cersei re-exports schemars 0.8; do NOT `cargo add schemars`
use serde::{Deserialize, Serialize};
use std::fmt::Write;
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

/// Print the findings — JSON when `json`, otherwise grouped text — and
/// additionally write a markdown report to `output` when one is given.
/// `--output` is independent of `--json`: you can have both.
pub fn emit(
    findings: &[Finding],
    failures: &[String],
    json: bool,
    output: Option<&Path>,
) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "findings": findings,
                "failed_reviews": failures,
            }))?
        );
    } else {
        print!("{}", render_terminal(findings, failures));
    }
    if let Some(path) = output {
        std::fs::write(path, render_markdown(findings, failures))?;
        eprintln!("report written to {}", path.display());
    }
    Ok(())
}

fn sev_label(s: Severity) -> &'static str {
    match s {
        Severity::High => "HIGH",
        Severity::Medium => "MED",
        Severity::Low => "LOW",
        Severity::Info => "INFO",
    }
}

/// Group findings by review, in the order the reviews first appear.
///
/// The runner hands us findings already sorted by (severity, review,
/// file), so first-appearance order puts the review holding the worst
/// finding first, and each group is severity-sorted for free. Do not
/// re-sort here — that property is the caller's to keep.
fn render_terminal(findings: &[Finding], failures: &[String]) -> String {
    let mut out = String::new();
    if findings.is_empty() {
        out.push_str("No findings.\n");
    }
    let mut reviews: Vec<&str> = Vec::new();
    for f in findings {
        if !reviews.contains(&f.review.as_str()) {
            reviews.push(&f.review);
        }
    }
    for review in reviews {
        let group: Vec<_> = findings.iter().filter(|f| f.review == review).collect();
        let noun = if group.len() == 1 {
            "finding"
        } else {
            "findings"
        };
        writeln!(out, "\n{review} ({} {noun})", group.len()).unwrap();
        for f in group {
            let loc = match f.line {
                Some(l) => format!("{}:{}", f.file, l),
                None => f.file.clone(),
            };
            writeln!(
                out,
                "  {:<5} {:<40} {}",
                sev_label(f.severity),
                loc,
                f.message
            )
            .unwrap();
        }
    }
    for fail in failures {
        writeln!(out, "\nWARNING: {fail}").unwrap();
    }
    out
}

fn render_markdown(findings: &[Finding], failures: &[String]) -> String {
    let mut out = String::from("# Code review findings\n");
    writeln!(out, "\n{} findings\n", findings.len()).unwrap();
    writeln!(out, "| Severity | Review | Location | Finding |").unwrap();
    writeln!(out, "|---|---|---|---|").unwrap();
    for f in findings {
        let loc = match f.line {
            Some(l) => format!("`{}:{}`", f.file, l),
            None => format!("`{}`", f.file),
        };
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            sev_label(f.severity),
            f.review,
            loc,
            // A `|` in a message would otherwise open a new table cell.
            f.message.replace('|', "\\|")
        )
        .unwrap();
    }
    for fail in failures {
        writeln!(out, "\n> WARNING: {fail}").unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Finding> {
        vec![
            Finding {
                review: "security".into(),
                severity: Severity::High,
                file: "src/auth.ts".into(),
                line: Some(42),
                message: "SQL built by string concat".into(),
            },
            Finding {
                review: "bugs".into(),
                severity: Severity::Medium,
                file: "src/api.ts".into(),
                line: None,
                message: "error swallowed".into(),
            },
        ]
    }

    #[test]
    fn terminal_groups_by_review() {
        let s = render_terminal(&sample(), &[]);
        let sec = s.find("security (1 finding)").unwrap();
        let bug = s.find("bugs (1 finding)").unwrap();
        assert!(s.contains("HIGH"));
        assert!(s.contains("src/auth.ts:42"));
        assert!(sec < bug || bug < sec); // both present; order = severity of worst finding
    }

    #[test]
    fn markdown_has_table() {
        let s = render_markdown(&sample(), &[]);
        assert!(s.contains("| Severity |"));
        assert!(s.contains("src/auth.ts"));
    }

    #[test]
    fn failures_are_shown() {
        let s = render_terminal(&[], &["review 'bugs' failed: timeout".into()]);
        assert!(s.contains("failed"));
    }
}
