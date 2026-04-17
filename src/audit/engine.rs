use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::graph::CodeGraph;
use crate::language::Language;
use crate::workspace::Workspace;

use super::models::{AuditFinding, AuditSummary};

pub struct AuditEngine {
    languages: Vec<Language>,
    pipeline_filter: Vec<String>,
    category_filter: Vec<String>,
    progress: Option<indicatif::ProgressBar>,
    project_dir: Option<std::path::PathBuf>,
}

impl Default for AuditEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditEngine {
    pub fn new() -> Self {
        Self {
            languages: vec![Language::Rust],
            pipeline_filter: Vec::new(),
            category_filter: Vec::new(),
            progress: None,
            project_dir: None,
        }
    }

    pub fn languages(mut self, langs: Vec<Language>) -> Self {
        self.languages = langs;
        self
    }

    pub fn pipelines(mut self, names: Vec<String>) -> Self {
        self.pipeline_filter = names;
        self
    }

    pub fn categories(mut self, cats: Vec<String>) -> Self {
        self.category_filter = cats;
        self
    }

    pub fn progress_bar(mut self, pb: indicatif::ProgressBar) -> Self {
        self.progress = Some(pb);
        self
    }

    pub fn project_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.project_dir = Some(dir);
        self
    }

    pub fn run(
        &self,
        workspace: &Workspace,
        graph: Option<&CodeGraph>,
    ) -> Result<(Vec<AuditFinding>, AuditSummary)> {
        // Discover JSON audit files (project-local → user-global → built-ins)
        let json_audits = crate::audit::json_audit::discover_json_audits(
            self.project_dir.as_deref(),
        );

        // No Rust pipelines remain — all audit logic is JSON-driven.
        // files_scanned counts workspace files visible to the engine's languages.
        let files_scanned = workspace
            .files()
            .iter()
            .filter(|rel_path| workspace.file_language(rel_path).is_some_and(|l| self.languages.contains(&l)))
            .count();

        if let Some(pb) = &self.progress {
            pb.set_length(files_scanned as u64);
            pb.finish_and_clear();
        }

        let mut findings: Vec<AuditFinding> = Vec::new();

        // Run JSON audit pipelines after Rust pipelines
        if let Some(g) = graph {
            for json_audit in &json_audits {
                // Apply category filter if set
                if !self.category_filter.is_empty()
                    && !self.category_filter.iter().any(|c| c == &json_audit.category)
                {
                    continue;
                }

                // Apply pipeline filter if set
                if !self.pipeline_filter.is_empty()
                    && !self.pipeline_filter.contains(&json_audit.pipeline)
                {
                    continue;
                }

                // Apply language filter: skip if none of the engine's languages match
                if let Some(ref langs) = json_audit.languages {
                    let matches = langs.iter().any(|lang_str| {
                        self.languages
                            .iter()
                            .any(|l| l.as_str().eq_ignore_ascii_case(lang_str))
                    });
                    if !matches {
                        continue;
                    }
                }

                match crate::graph::executor::run_pipeline(
                    &json_audit.graph,
                    g,
                    Some(workspace),
                    json_audit.languages.as_deref(),
                    None,
                    &json_audit.pipeline,
                ) {
                    Ok(crate::graph::executor::PipelineOutput::Findings(new_findings)) => {
                        findings.extend(new_findings);
                    }
                    Ok(crate::graph::executor::PipelineOutput::Results(_)) => {
                        // Non-flag pipelines in audit context don't produce findings
                    }
                    Err(e) => {
                        eprintln!("Warning: JSON audit '{}' failed: {e}", json_audit.pipeline);
                    }
                }
            }
        }

        let summary = compute_summary(&findings, files_scanned);

        Ok((findings, summary))
    }
}

/// Phase 4.2: Single-pass summary computation with sort_unstable_by.
fn compute_summary(findings: &[AuditFinding], files_scanned: usize) -> AuditSummary {
    let mut files_seen: HashSet<&str> = HashSet::new();
    let mut by_pipeline: HashMap<String, usize> = HashMap::new();
    let mut by_pattern: HashMap<String, usize> = HashMap::new();
    let mut pipeline_pattern: HashMap<String, HashMap<String, usize>> = HashMap::new();

    for f in findings {
        files_seen.insert(&f.file_path);
        *by_pipeline.entry(f.pipeline.clone()).or_default() += 1;
        *by_pattern.entry(f.pattern.clone()).or_default() += 1;
        *pipeline_pattern
            .entry(f.pipeline.clone())
            .or_default()
            .entry(f.pattern.clone())
            .or_default() += 1;
    }

    let mut by_pipeline: Vec<(String, usize)> = by_pipeline.into_iter().collect();
    by_pipeline.sort_unstable_by(|a, b| b.1.cmp(&a.1));

    let mut by_pattern: Vec<(String, usize)> = by_pattern.into_iter().collect();
    by_pattern.sort_unstable_by(|a, b| b.1.cmp(&a.1));

    let by_pipeline_pattern: Vec<(String, Vec<(String, usize)>)> = by_pipeline
        .iter()
        .map(|(pipeline_name, _)| {
            let mut patterns: Vec<(String, usize)> = pipeline_pattern
                .remove(pipeline_name)
                .unwrap_or_default()
                .into_iter()
                .collect();
            patterns.sort_unstable_by(|a, b| b.1.cmp(&a.1));
            (pipeline_name.clone(), patterns)
        })
        .collect();

    AuditSummary {
        total_findings: findings.len(),
        files_scanned,
        files_with_findings: files_seen.len(),
        by_pipeline,
        by_pattern,
        by_pipeline_pattern,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    #[test]
    fn engine_basic() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("test.rs"),
            r#"fn main() { Some(1).unwrap(); panic!("x"); }"#,
        )
        .unwrap();

        let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
        let graph = crate::graph::builder::GraphBuilder::new(&workspace, &[Language::Rust])
            .build()
            .unwrap();
        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(&workspace, Some(&graph))
            .unwrap();

        // JSON pipelines (panic_detection, async_blocking, etc.) fire on method/scoped calls.
        // Exact count varies by pipeline set; verify at least 1 finding is produced.
        // Note: files_scanned counts only Rust-lang-pipeline files (0 now that all Rust
        // tech-debt pipelines are JSON); files_with_findings counts files from JSON findings.
        assert!(
            findings.len() >= 1,
            "expected at least 1 finding from JSON pipelines; got 0"
        );
        assert_eq!(summary.total_findings, findings.len());
        assert_eq!(summary.files_with_findings, 1);
    }

    #[test]
    fn engine_pipeline_filter() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("test.rs"),
            r#"fn main() { Some(1).unwrap(); }"#,
        )
        .unwrap();

        let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
        let (findings, _) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .pipelines(vec!["nonexistent_pipeline".to_string()])
            .run(&workspace, None)
            .unwrap();

        assert!(findings.is_empty());
    }

    #[test]
    fn engine_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");

        let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(&workspace, None)
            .unwrap();

        assert!(findings.is_empty());
        assert_eq!(summary.files_scanned, 0);
    }

    #[test]
    fn engine_skips_non_rust() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("test.ts"), "const x = something.unwrap();").unwrap();

        let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(&workspace, None)
            .unwrap();

        assert!(findings.is_empty());
        assert_eq!(summary.files_scanned, 0);
    }

    /// JSON audit findings appear in the output when a graph is supplied.
    #[test]
    fn engine_json_audit_findings_merged() {
        use crate::graph::builder::GraphBuilder;

        // Source dir: one Rust file
        let src_dir = tempfile::tempdir().expect("src_dir");
        std::fs::write(src_dir.path().join("lib.rs"), "fn main() {}").unwrap();

        let workspace =
            Workspace::load(src_dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
        let graph = GraphBuilder::new(&workspace, &[Language::Rust])
            .build()
            .expect("graph build");

        // Project dir: contains a .virgil/audits/ JSON audit that flags every file
        let proj_dir = tempfile::tempdir().expect("proj_dir");
        let audit_dir = proj_dir.path().join(".virgil").join("audits");
        std::fs::create_dir_all(&audit_dir).unwrap();
        let audit_json = r#"{
            "pipeline": "always_flag_test",
            "category": "architecture",
            "description": "test: flag every file",
            "graph": [
                {"select": "file"},
                {"flag": {"pattern": "test_flag", "message": "flagged {{file}}", "severity": "info"}}
            ]
        }"#;
        std::fs::write(audit_dir.join("always_flag_test.json"), audit_json).unwrap();

        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .categories(vec!["architecture".to_string()])
            .project_dir(proj_dir.path().to_path_buf())
            .run(&workspace, Some(&graph))
            .unwrap();

        // The JSON audit should have flagged lib.rs
        assert!(
            findings.iter().any(|f| f.pipeline == "always_flag_test"),
            "expected findings from JSON audit; got: {:?}",
            findings.iter().map(|f| &f.pipeline).collect::<Vec<_>>()
        );
        assert!(summary.total_findings > 0);
    }

    /// A project-local JSON audit named "cross_file_coupling" runs and produces findings
    /// (all architecture analysis is now JSON-only; no Rust ProjectAnalyzer to override).
    #[test]
    fn engine_json_audit_cross_file_coupling_runs() {
        // Build a project dir with a JSON audit for cross_file_coupling
        let proj_dir = tempfile::tempdir().expect("proj_dir");
        let audit_dir = proj_dir.path().join(".virgil").join("audits");
        std::fs::create_dir_all(&audit_dir).unwrap();
        let override_json = r#"{
            "pipeline": "cross_file_coupling",
            "category": "architecture",
            "description": "JSON cross_file_coupling",
            "graph": [
                {"select": "file"},
                {"flag": {"pattern": "json_coupling", "message": "json coupling {{file}}", "severity": "info"}}
            ]
        }"#;
        std::fs::write(
            audit_dir.join("cross_file_coupling.json"),
            override_json,
        )
        .unwrap();

        let src_dir = tempfile::tempdir().expect("src_dir");
        std::fs::write(src_dir.path().join("a.rs"), "fn foo() {}").unwrap();
        let workspace =
            Workspace::load(src_dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
        let graph = crate::graph::builder::GraphBuilder::new(&workspace, &[Language::Rust])
            .build()
            .expect("graph build");

        let (findings, _) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .categories(vec!["architecture".to_string()])
            .project_dir(proj_dir.path().to_path_buf())
            .run(&workspace, Some(&graph))
            .unwrap();

        // The JSON pipeline should produce json_coupling findings
        let json_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pipeline == "cross_file_coupling" && f.pattern == "json_coupling")
            .collect();
        assert!(
            !json_findings.is_empty(),
            "expected json_coupling findings from the JSON pipeline"
        );
    }
}
