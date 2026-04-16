use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use rayon::prelude::*;

use crate::graph::CodeGraph;
use crate::language::Language;
use crate::parser;
use crate::workspace::Workspace;

use super::analyzers;
use super::models::{AuditFinding, AuditSummary};
use super::pipeline::{self, AnyPipeline, GraphPipelineContext, PipelineContext};
use super::pipelines::helpers::count_all_identifier_occurrences;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineSelector {
    TechDebt,
    Complexity,
    CodeStyle,
    Security,
    Scalability,
    Architecture,
}

pub struct AuditEngine {
    languages: Vec<Language>,
    pipeline_filter: Vec<String>,
    pipeline_selector: PipelineSelector,
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
            pipeline_selector: PipelineSelector::TechDebt,
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

    pub fn pipeline_selector(mut self, s: PipelineSelector) -> Self {
        self.pipeline_selector = s;
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

        // Build a set of pipeline names that JSON audits override
        let json_pipeline_names: std::collections::HashSet<String> = json_audits
            .iter()
            .map(|a| a.pipeline.clone())
            .collect();

        // Build pipelines per language, apply filter
        let mut pipeline_map: HashMap<Language, Vec<Arc<AnyPipeline>>> = HashMap::new();
        for lang in &self.languages {
            let mut lang_pipelines = match self.pipeline_selector {
                PipelineSelector::TechDebt => pipeline::pipelines_for_language(*lang)?,
                PipelineSelector::Complexity => pipeline::complexity_pipelines_for_language(*lang)?,
                PipelineSelector::CodeStyle => pipeline::code_style_pipelines_for_language(*lang)?,
                PipelineSelector::Security => pipeline::security_pipelines_for_language(*lang)?,
                PipelineSelector::Scalability => {
                    pipeline::scalability_pipelines_for_language(*lang)?
                }
                PipelineSelector::Architecture => {
                    // All architecture pipelines are now JSON-only
                    vec![]
                }
            };

            // ENG-01: suppress Rust lang_pipelines that are overridden by a JSON pipeline
            lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()));

            if !self.pipeline_filter.is_empty() {
                lang_pipelines.retain(|p| self.pipeline_filter.contains(&p.name().to_string()));
            }

            if !lang_pipelines.is_empty() {
                let arced: Vec<Arc<AnyPipeline>> =
                    lang_pipelines.into_iter().map(Arc::new).collect();
                pipeline_map.insert(*lang, arced);
            }
        }

        let pipeline_map = Arc::new(pipeline_map);

        // Group workspace files by language
        let grouped_files: Vec<(Language, &str)> = workspace
            .files()
            .iter()
            .filter_map(|rel_path| {
                let lang = workspace.file_language(rel_path)?;
                if pipeline_map.contains_key(&lang) {
                    Some((lang, rel_path.as_str()))
                } else {
                    None
                }
            })
            .collect();

        let files_scanned = grouped_files.len();

        if let Some(pb) = &self.progress {
            pb.set_length(files_scanned as u64);
        }

        let progress = self.progress.clone();
        // Provide a fallback empty graph so Graph pipelines run even when the
        // caller does not supply a pre-built CodeGraph (e.g. tech-debt audit
        // without explicit graph construction).
        let fallback_graph = CodeGraph::new();
        let effective_graph: &CodeGraph = graph.unwrap_or(&fallback_graph);
        let graph_ref = graph;

        // Phase 4.4: Reduced stack size — stack-based iteration in helpers
        // eliminates deep recursion, so 4MB suffices.
        let pool = rayon::ThreadPoolBuilder::new()
            .stack_size(4 * 1024 * 1024) // 4MB per thread (reduced from 16MB)
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

        // Run pipelines in parallel over pre-grouped files
        let all_findings: Vec<Vec<AuditFinding>> = pool.install(|| {
            grouped_files
                .par_iter()
                .filter_map(|&(lang, rel_path)| {
                    let result = (|| {
                        let pipelines = pipeline_map.get(&lang)?;

                        let mut ts_parser = match parser::create_parser(lang) {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!("Warning: failed to create parser for {}: {e}", rel_path);
                                return None;
                            }
                        };

                        let source = workspace.read_file(rel_path)?;

                        let tree = match ts_parser.parse(&*source, None) {
                            Some(t) => t,
                            None => {
                                eprintln!("Warning: failed to parse {}", rel_path);
                                return None;
                            }
                        };

                        let id_counts =
                            count_all_identifier_occurrences(tree.root_node(), source.as_bytes());

                        let mut file_findings = Vec::new();
                        for pipeline in pipelines {
                            match pipeline.as_ref() {
                                AnyPipeline::Node(p) => {
                                    file_findings.extend(p.check(
                                        &tree,
                                        source.as_bytes(),
                                        rel_path,
                                    ));
                                }
                                AnyPipeline::Graph(p) => {
                                    let ctx = GraphPipelineContext {
                                        tree: &tree,
                                        source: source.as_bytes(),
                                        file_path: rel_path,
                                        id_counts: &id_counts,
                                        graph: effective_graph,
                                    };
                                    file_findings.extend(p.check(&ctx));
                                }
                                AnyPipeline::Legacy(p) => {
                                    let ctx = PipelineContext {
                                        tree: &tree,
                                        source: source.as_bytes(),
                                        file_path: rel_path,
                                        id_counts: &id_counts,
                                        graph: graph_ref,
                                    };
                                    file_findings.extend(p.check_with_context(&ctx));
                                }
                            }
                        }

                        Some(file_findings)
                    })();
                    if let Some(pb) = &progress {
                        pb.inc(1);
                    }
                    result
                })
                .collect()
        });

        if let Some(pb) = &self.progress {
            pb.finish_and_clear();
        }

        let mut findings: Vec<AuditFinding> = all_findings.into_iter().flatten().collect();

        // Run project-level analyzers if graph is provided
        if let Some(g) = graph {
            let mut project_analyzers: Vec<Box<dyn super::project_analyzer::ProjectAnalyzer>> =
                match self.pipeline_selector {
                    PipelineSelector::Architecture => analyzers::architecture_analyzers(),
                    PipelineSelector::CodeStyle => analyzers::code_style_analyzers(),
                    _ => Vec::new(),
                };

            // JSON audits override Rust analyzers with the same pipeline name
            project_analyzers.retain(|a| !json_pipeline_names.contains(a.name()));

            if !self.pipeline_filter.is_empty() {
                project_analyzers.retain(|a| self.pipeline_filter.contains(&a.name().to_string()));
            }

            for analyzer in &project_analyzers {
                findings.extend(analyzer.analyze(g));
            }
        }

        // Run JSON audit pipelines after Rust pipelines
        if let Some(g) = graph {
            for json_audit in &json_audits {
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
        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(&workspace, None)
            .unwrap();

        assert_eq!(findings.len(), 2);
        assert_eq!(summary.total_findings, 2);
        assert_eq!(summary.files_scanned, 1);
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
            .pipeline_selector(PipelineSelector::Architecture)
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

    /// When a project-local JSON audit has the same pipeline name as a built-in Rust
    /// ProjectAnalyzer, the Rust analyzer is skipped (JSON wins).
    #[test]
    fn engine_json_audit_overrides_rust_project_analyzer() {
        use crate::audit::analyzers;

        // Verify that "cross_file_coupling" is among the architecture analyzers
        // (so we know we're testing a real override of a Rust ProjectAnalyzer).
        let arch_analyzers = analyzers::architecture_analyzers();
        let arch_names: Vec<&str> = arch_analyzers.iter().map(|a| a.name()).collect();
        assert!(
            arch_names.contains(&"cross_file_coupling"),
            "precondition: cross_file_coupling must be an architecture analyzer"
        );

        // Build a project dir with a JSON audit that overrides cross_file_coupling
        let proj_dir = tempfile::tempdir().expect("proj_dir");
        let audit_dir = proj_dir.path().join(".virgil").join("audits");
        std::fs::create_dir_all(&audit_dir).unwrap();
        let override_json = r#"{
            "pipeline": "cross_file_coupling",
            "category": "architecture",
            "description": "JSON override of cross_file_coupling",
            "graph": [
                {"select": "file"},
                {"flag": {"pattern": "json_coupling", "message": "json override {{file}}", "severity": "info"}}
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
            .pipeline_selector(PipelineSelector::Architecture)
            .project_dir(proj_dir.path().to_path_buf())
            .run(&workspace, Some(&graph))
            .unwrap();

        // The JSON override should produce json_coupling findings, not the Rust analyzer output
        let json_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pipeline == "cross_file_coupling" && f.pattern == "json_coupling")
            .collect();
        assert!(
            !json_findings.is_empty(),
            "expected json_coupling findings from the JSON override"
        );
    }
}
