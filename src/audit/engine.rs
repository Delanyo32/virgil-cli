use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::discovery;
use crate::language::Language;
use crate::parser;

use super::models::{AuditFinding, AuditSummary};
use super::pipeline::{self, Pipeline};

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
}

impl AuditEngine {
    pub fn new() -> Self {
        Self {
            languages: vec![Language::Rust],
            pipeline_filter: Vec::new(),
            pipeline_selector: PipelineSelector::TechDebt,
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

    pub fn run(&self, root: &Path) -> Result<(Vec<AuditFinding>, AuditSummary)> {
        let root = root
            .canonicalize()
            .with_context(|| format!("invalid directory: {}", root.display()))?;

        let files = discovery::discover_files(&root, &self.languages)?;

        // Build pipelines per language, apply filter
        let mut pipeline_map: HashMap<Language, Vec<Arc<dyn Pipeline>>> = HashMap::new();
        for lang in &self.languages {
            let mut lang_pipelines = match self.pipeline_selector {
                PipelineSelector::TechDebt => pipeline::pipelines_for_language(*lang)?,
                PipelineSelector::Complexity => pipeline::complexity_pipelines_for_language(*lang)?,
                PipelineSelector::CodeStyle => pipeline::code_style_pipelines_for_language(*lang)?,
                PipelineSelector::Security => pipeline::security_pipelines_for_language(*lang)?,
                PipelineSelector::Scalability => pipeline::scalability_pipelines_for_language(*lang)?,
                PipelineSelector::Architecture => pipeline::architecture_pipelines_for_language(*lang)?,
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

        // Run pipelines in parallel over files
        let all_findings: Vec<Vec<AuditFinding>> = files
            .par_iter()
            .filter_map(|path| {
                let ext = path.extension()?.to_str()?;
                let lang = Language::from_extension(ext)?;
                let pipelines = pipeline_map.get(&lang)?;

                let mut ts_parser = match parser::create_parser(lang) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!(
                            "Warning: failed to create parser for {}: {e}",
                            path.display()
                        );
                        return None;
                    }
                };

                let source = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Warning: failed to read {}: {e}", path.display());
                        return None;
                    }
                };

                let tree = match ts_parser.parse(&source, None) {
                    Some(t) => t,
                    None => {
                        eprintln!("Warning: failed to parse {}", path.display());
                        return None;
                    }
                };

                let relative_path = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");

                let mut file_findings = Vec::new();
                for pipeline in pipelines {
                    file_findings.extend(pipeline.check(
                        &tree,
                        source.as_bytes(),
                        &relative_path,
                    ));
                }

                Some(file_findings)
            })
            .collect();

        let findings: Vec<AuditFinding> = all_findings.into_iter().flatten().collect();

        // Compute summary
        let files_with_findings = {
            let mut seen = std::collections::HashSet::new();
            for f in &findings {
                seen.insert(f.file_path.clone());
            }
            seen.len()
        };

        let mut by_pipeline: HashMap<String, usize> = HashMap::new();
        let mut by_pattern: HashMap<String, usize> = HashMap::new();
        let mut pipeline_pattern_map: HashMap<String, HashMap<String, usize>> = HashMap::new();
        for f in &findings {
            *by_pipeline.entry(f.pipeline.clone()).or_insert(0) += 1;
            *by_pattern.entry(f.pattern.clone()).or_insert(0) += 1;
            *pipeline_pattern_map
                .entry(f.pipeline.clone())
                .or_default()
                .entry(f.pattern.clone())
                .or_insert(0) += 1;
        }

        let mut by_pipeline: Vec<(String, usize)> = by_pipeline.into_iter().collect();
        by_pipeline.sort_by(|a, b| b.1.cmp(&a.1));

        let mut by_pattern: Vec<(String, usize)> = by_pattern.into_iter().collect();
        by_pattern.sort_by(|a, b| b.1.cmp(&a.1));

        let by_pipeline_pattern: Vec<(String, Vec<(String, usize)>)> = by_pipeline
            .iter()
            .map(|(pipeline_name, _)| {
                let mut patterns: Vec<(String, usize)> = pipeline_pattern_map
                    .remove(pipeline_name)
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                patterns.sort_by(|a, b| b.1.cmp(&a.1));
                (pipeline_name.clone(), patterns)
            })
            .collect();

        let summary = AuditSummary {
            total_findings: findings.len(),
            files_scanned: files.len(),
            files_with_findings,
            by_pipeline,
            by_pattern,
            by_pipeline_pattern,
        };

        Ok((findings, summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_basic() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("test.rs"),
            r#"fn main() { Some(1).unwrap(); panic!("x"); }"#,
        )
        .unwrap();

        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(dir.path())
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

        let (findings, _) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .pipelines(vec!["nonexistent_pipeline".to_string()])
            .run(dir.path())
            .unwrap();

        assert!(findings.is_empty());
    }

    #[test]
    fn engine_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");

        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(dir.path())
            .unwrap();

        assert!(findings.is_empty());
        assert_eq!(summary.files_scanned, 0);
    }

    #[test]
    fn engine_skips_non_rust() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("test.ts"),
            "const x = something.unwrap();",
        )
        .unwrap();

        let (findings, summary) = AuditEngine::new()
            .languages(vec![Language::Rust])
            .run(dir.path())
            .unwrap();

        assert!(findings.is_empty());
        assert_eq!(summary.files_scanned, 0);
    }
}
