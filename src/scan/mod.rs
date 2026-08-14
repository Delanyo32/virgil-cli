pub mod prompts;
pub mod provider;
pub mod report;
pub mod tools;

use crate::cli::ProviderKind;
use crate::scan::report::Finding;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{info, warn};

/// Turns an agent may take before cersei stops it. cersei's default is 10,
/// which is low for an agent that queries, reads source, then reports.
///
/// ponytail: a guess, not a measurement — tune it from a real scan.
const MAX_TURNS: u32 = 30;

/// Wall-clock cap on one review. Hitting it fails that review only: the
/// findings it already reported still travel back, and the other reviews
/// are untouched.
///
/// This is the *second* layer of hang protection. The first is
/// `run_stream` below, which fixes the deadlock. This one covers a
/// provider that accepts the request and then stops answering, because
/// cersei builds its HTTP client with `reqwest::Client::new()`, which
/// sets no read timeout.
const REVIEW_TIMEOUT: Duration = Duration::from_secs(600);

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
    // Everything that can fail on bad input runs before the parse, so a
    // missing API key or an empty --prompts directory costs a second,
    // not a full repository parse. The provider built here is thrown
    // away; each review gets its own below.
    let prompts = prompts::load(cfg.prompts.as_deref())?;
    let model = provider::resolve_model(cfg.provider, cfg.model.as_deref())?;
    drop(provider::build(cfg.provider, &model)?);

    let (store, _ws) = build_store(&cfg)?;
    let system = prompts::system_prompt();

    let sem = Arc::new(Semaphore::new(cfg.workers.max(1)));
    let mut handles = Vec::new();

    for prompt in prompts {
        // `try_clone_store` runs here, before the spawn, because it must
        // be called on the thread holding the original store.
        let agent_store = store.try_clone_store()?;
        let provider = provider::build(cfg.provider, &model)?;

        // Per-agent sink: the tools push into it, the runner drains it.
        let sink: Arc<Mutex<Vec<Finding>>> = Arc::default();
        let sem = sem.clone();
        let system = system.clone();
        let model = model.clone();

        // Kept outside the task so a panicked task can still be named.
        let label = prompt.name.clone();

        handles.push((
            label,
            tokio::spawn(async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .expect("the semaphore is never closed");
                let name = prompt.name;
                info!(review = %name, "review started");

                let (query, read_source, report_finding) =
                    tools::make_tools(agent_store, sink.clone());

                // `run_stream`, NOT `run_with` — this is load-bearing.
                // cersei 0.2.6's `run_with` calls `run_agent`, which
                // creates `mpsc::channel(512)` and binds the receiving
                // end to `_event_rx` (runner.rs:147). An underscore
                // *prefix* still holds the receiver alive for the whole
                // function, and nothing ever reads it, so the channel
                // fills and `event_tx.send().await` (runner.rs:381)
                // blocks forever. A single normal reply streamed 2,351
                // chunks against a 512-slot channel, so this deadlocked
                // on every provider. `run_stream` hands us the receiver
                // instead, and `collect` drains it while the agent runs.
                let run = async {
                    match cersei::Agent::builder()
                        .provider_boxed(provider)
                        .system_prompt(&system)
                        .model(&model)
                        .max_turns(MAX_TURNS)
                        .tool(query)
                        .tool(read_source)
                        .tool(report_finding)
                        .build()
                    {
                        // `run_stream` takes `&Arc<Agent>` because it
                        // spawns the loop as its own task.
                        Ok(agent) => Arc::new(agent).run_stream(&prompt.body).collect().await,
                        Err(e) => Err(e),
                    }
                };
                let out = tokio::time::timeout(REVIEW_TIMEOUT, run).await;

                // Drained on every path: an agent that errors or stalls
                // mid-run usually breaks *after* it has reported real,
                // confirmed findings (rate limit, dropped connection),
                // so they travel back with the failure instead of being
                // thrown away.
                let batch = std::mem::take(&mut *sink.lock().unwrap());
                let failure = match out {
                    Ok(Ok(_)) => {
                        info!(review = %name, findings = batch.len(), "review finished");
                        None
                    }
                    // One failed review must not sink the scan.
                    Ok(Err(e)) => {
                        warn!(review = %name, findings = batch.len(), error = %e, "review failed");
                        Some(format!("review '{name}' failed: {e}"))
                    }
                    // ponytail: the abandoned agent task runs on to its
                    // own end — cersei 0.2.6 ignores the control channel
                    // it hands back, so `AgentStream::cancel` is a no-op.
                    // Dropping the stream does un-block it (its sends
                    // start failing instead of waiting), so this costs
                    // tokens, not a stuck process.
                    Err(_) => {
                        let secs = REVIEW_TIMEOUT.as_secs();
                        warn!(review = %name, findings = batch.len(), secs, "review timed out");
                        Some(format!("review '{name}' timed out after {secs}s"))
                    }
                };
                (name, batch, failure)
            }),
        ));
    }

    let mut outcomes = Vec::with_capacity(handles.len());
    for (label, handle) in handles {
        match handle.await {
            Ok(outcome) => outcomes.push(outcome),
            // A `JoinError` means the agent task panicked. That is a bug,
            // but aborting here would throw away every other review's
            // findings and print nothing, so record it like any other
            // failed review and carry on.
            Err(e) => {
                warn!(review = %label, error = %e, "review panicked");
                let failure = format!("review '{label}' panicked");
                outcomes.push((label, Vec::new(), Some(failure)));
            }
        }
    }
    let (mut findings, failures) = collect(outcomes);
    sort_findings(&mut findings);
    report::emit(&findings, &failures, cfg.json, cfg.output.as_deref())?;

    if scan_failed(findings.len(), failures.len()) {
        anyhow::bail!("no findings, and {} review(s) failed", failures.len());
    }
    Ok(())
}

/// One review's outcome: its name, whatever it reported, and the failure
/// message if it broke. A review can contribute both findings and a
/// failure — the two are independent.
type Outcome = (String, Vec<Finding>, Option<String>);

/// Fold every review's outcome into one finding list plus the failure
/// messages. Findings reported before a review broke are kept and tagged
/// exactly like a clean review's.
fn collect(outcomes: Vec<Outcome>) -> (Vec<Finding>, Vec<String>) {
    let mut findings = Vec::new();
    let mut failures = Vec::new();
    for (name, batch, failure) in outcomes {
        merge(&mut findings, batch, &name);
        failures.extend(failure);
    }
    (findings, failures)
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

/// The scan fails when nothing was reported *and* at least one review
/// broke. Any finding at all beats a partial failure, so a scan that
/// reported something exits 0 even though a review died.
///
/// Note the corollary: on a genuinely clean repo, one broken review is
/// enough to fail the scan, because there are no findings to outweigh
/// it. The bail message counts the broken reviews rather than claiming
/// they all broke, because in that case most of them did not.
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

    /// A review that breaks mid-run keeps what it already reported. The
    /// error usually lands after real findings, so dropping them can empty
    /// a scan that actually found things.
    #[test]
    fn a_failed_review_still_contributes_its_findings() {
        let (findings, failures) = collect(vec![
            (
                "bugs".into(),
                vec![
                    finding(Severity::High, "a.rs"),
                    finding(Severity::Low, "b.rs"),
                ],
                Some("review 'bugs' failed: rate limited".into()),
            ),
            ("security".into(), vec![], None),
        ]);
        assert_eq!(findings.len(), 2, "both partial findings survive");
        assert!(
            findings.iter().all(|f| f.review == "bugs"),
            "partial findings are tagged like any other"
        );
        assert_eq!(failures.len(), 1, "the failure is still reported");
        // The scan itself must not bail: it has findings to print.
        assert!(!scan_failed(findings.len(), failures.len()));
    }

    /// One broken review must not sink a scan that still found something.
    #[test]
    fn only_an_all_failed_scan_is_an_error() {
        assert!(scan_failed(0, 1), "nothing found and a review broke");
        assert!(!scan_failed(3, 1), "other reviews still reported");
        assert!(!scan_failed(0, 0), "clean codebase is not a failure");
    }
}
