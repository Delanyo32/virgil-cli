pub mod prompts;
pub mod provider;
pub mod report;
pub mod tools;

use crate::cli::ProviderKind;
use crate::scan::report::Finding;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tracing::{info, warn};

/// Turns an agent may take before cersei stops it. cersei's default is 10,
/// which is low for an agent that queries, reads source, then reports.
///
/// ponytail: a guess, not a measurement — tune it from a real scan.
const MAX_TURNS: u32 = 30;

pub struct ScanConfig {
    pub path: PathBuf,
    pub prompts: Option<PathBuf>,
    pub workers: usize,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub json: bool,
    pub output: Option<PathBuf>,
    pub rebuild: bool,
    pub lang: Option<String>,
}

/// Parse the project, then run one review agent per prompt, at most
/// `cfg.workers` at a time. A review that fails is recorded and skipped;
/// only an all-failed scan is an error.
pub async fn run(cfg: ScanConfig) -> Result<()> {
    let (store, _ws) = build_store(&cfg)?;
    let prompts = prompts::load(cfg.prompts.as_deref())?;
    let model = provider::resolve_model(cfg.provider, cfg.model.as_deref())?;
    let system = prompts::system_prompt();

    let sem = Arc::new(Semaphore::new(cfg.workers.max(1)));
    let mut handles = Vec::new();

    for prompt in prompts {
        // Both of these run here, before the spawn, on purpose:
        // `try_clone_store` must be called on the thread holding the
        // original store, and building the provider here makes a missing
        // API key fail the scan once instead of once per review.
        let agent_store = store.try_clone_store()?;
        let provider = provider::build(cfg.provider, &model)?;

        // Per-agent sink: the tools push into it, the runner drains it.
        let sink: Arc<Mutex<Vec<Finding>>> = Arc::default();
        let sem = sem.clone();
        let system = system.clone();
        let model = model.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem
                .acquire_owned()
                .await
                .expect("the semaphore is never closed");
            let name = prompt.name;
            info!(review = %name, "review started");

            let (query, read_source, report_finding) = tools::make_tools(agent_store, sink.clone());
            let out = cersei::Agent::builder()
                .provider_boxed(provider)
                .system_prompt(&system)
                .model(&model)
                .max_turns(MAX_TURNS)
                .tool(query)
                .tool(read_source)
                .tool(report_finding)
                .run_with(&prompt.body)
                .await;

            let batch = std::mem::take(&mut *sink.lock().unwrap());
            match out {
                Ok(_) => {
                    info!(review = %name, findings = batch.len(), "review finished");
                    Ok((name, batch))
                }
                // One failed review must not sink the scan.
                Err(e) => {
                    warn!(review = %name, error = %e, "review failed");
                    Err(format!("review '{name}' failed: {e}"))
                }
            }
        }));
    }

    let mut findings = Vec::new();
    let mut failures = Vec::new();
    for handle in handles {
        // A `JoinError` means an agent task panicked — that is a bug, not
        // a failed review, so it aborts the scan.
        match handle.await? {
            Ok((name, batch)) => merge(&mut findings, batch, &name),
            Err(e) => failures.push(e),
        }
    }
    sort_findings(&mut findings);
    report::emit(&findings, &failures, cfg.json, cfg.output.as_deref())?;

    if scan_failed(findings.len(), failures.len()) {
        anyhow::bail!("all reviews failed");
    }
    Ok(())
}

/// Stamp the review's name onto each finding and append it. The agent
/// never sets `review` itself, so the runner owns that field.
fn merge(all: &mut Vec<Finding>, batch: Vec<Finding>, review: &str) {
    all.extend(batch.into_iter().map(|mut f| {
        f.review = review.to_string();
        f
    }));
}

/// `High` first, then by review name, then by file. `Severity` declares
/// its variants worst-first, so an ascending sort is already the right
/// order.
fn sort_findings(findings: &mut [Finding]) {
    findings
        .sort_by(|a, b| (a.severity, &a.review, &a.file).cmp(&(b.severity, &b.review, &b.file)));
}

/// The scan itself fails only when every review broke and nothing came
/// back. Findings from the reviews that worked beat a partial failure.
fn scan_failed(findings: usize, failures: usize) -> bool {
    findings == 0 && failures > 0
}

pub(crate) fn build_store(
    cfg: &ScanConfig,
) -> Result<(crate::db::DbStore, crate::storage::workspace::Workspace)> {
    use crate::language::{self, Language};
    let root = cfg.path.canonicalize()?;
    let languages = match &cfg.lang {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };
    let ws = crate::storage::workspace::Workspace::load(&root, &languages, None)?;

    let project_id = root.to_string_lossy().to_string();
    let cache_path = crate::db::cache_dir_for_db(&project_id)?;
    if cfg.rebuild && cache_path.exists() {
        std::fs::remove_file(&cache_path)?;
    }
    let store = crate::db::DbStore::open_persistent(&cache_path)?;
    if store.fresh() {
        crate::graph::builder::GraphBuilder::new(&ws, &languages).build(&store)?;
        crate::db::populate(&store)?;
    }
    Ok((store, ws))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::report::Severity;

    fn finding(severity: Severity, file: &str) -> Finding {
        Finding {
            review: String::new(),
            severity,
            file: file.to_string(),
            line: None,
            message: "m".into(),
        }
    }

    #[test]
    fn merge_stamps_the_review_name() {
        let mut all = Vec::new();
        merge(&mut all, vec![finding(Severity::Low, "a.rs")], "security");
        merge(&mut all, vec![finding(Severity::Low, "b.rs")], "bugs");
        let names: Vec<_> = all.iter().map(|f| f.review.as_str()).collect();
        assert_eq!(names, ["security", "bugs"]);
    }

    #[test]
    fn sort_puts_high_first_then_review_then_file() {
        let mut all = Vec::new();
        merge(&mut all, vec![finding(Severity::Info, "z.rs")], "bugs");
        merge(&mut all, vec![finding(Severity::High, "b.rs")], "security");
        merge(&mut all, vec![finding(Severity::High, "a.rs")], "security");
        merge(&mut all, vec![finding(Severity::High, "c.rs")], "bugs");
        merge(&mut all, vec![finding(Severity::Medium, "y.rs")], "bugs");
        sort_findings(&mut all);
        let order: Vec<_> = all
            .iter()
            .map(|f| (f.severity, f.review.as_str(), f.file.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                (Severity::High, "bugs", "c.rs"),
                (Severity::High, "security", "a.rs"),
                (Severity::High, "security", "b.rs"),
                (Severity::Medium, "bugs", "y.rs"),
                (Severity::Info, "bugs", "z.rs"),
            ]
        );
    }

    /// One broken review must not sink a scan that still found something.
    #[test]
    fn only_an_all_failed_scan_is_an_error() {
        assert!(scan_failed(0, 1), "nothing found and a review broke");
        assert!(!scan_failed(3, 1), "other reviews still reported");
        assert!(!scan_failed(0, 0), "clean codebase is not a failure");
    }
}
