use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use rayon::prelude::*;

use crate::language::Language;
use crate::parser;
use crate::workspace::Workspace;

use super::models::{AuditFinding, AuditSummary};
use super::pipeline::{self, Pipeline};
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

    pub fn run(&self, workspace: &Workspace) -> Result<(Vec<AuditFinding>, AuditSummary)> {
        // Build pipelines per language, apply filter
        let mut pipeline_map: HashMap<Language, Vec<Arc<dyn Pipeline>>> = HashMap::new();
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
                    pipeline::architecture_pipelines_for_language(*lang)?
                }
            };

            if !self.pipeline_filter.is_empty() {
                lang_pipelines.retain(|p| self.pipeline_filter.contains(&p.name().to_string()));
            }

            if !lang_pipelines.is_empty() {
                let arced: Vec<Arc<dyn Pipeline>> =
                    lang_pipelines.into_iter().map(Arc::from).collect();
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
                                eprintln!(
                                    "Warning: failed to create parser for {}: {e}",
                                    rel_path
                                );
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
                            file_findings.extend(pipeline.check_with_ids(
                                &tree,
                                source.as_bytes(),
                                rel_path,
                                &id_counts,
                            ));
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

        let findings: Vec<AuditFinding> = all_findings.into_iter().flatten().collect();
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
            .run(&workspace)
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
            .run(&workspace)
            .unwrap();

        assert!(findings.is_empty());
    }

    #[test]
    fn engine_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");

        let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(&workspace)
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
            .run(&workspace)
            .unwrap();

        assert!(findings.is_empty());
        assert_eq!(summary.files_scanned, 0);
    }
}
