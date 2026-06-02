//! Serve mode: expose an already-parsed project over a local HTTP API.
//!
//! Read-only. The project's warm DuckDB store must already exist (serve
//! never builds); queries run as async jobs against a pool of sibling
//! connections. See `docs/superpowers/plans/2026-06-02-serve-mode.md`.

mod http;
mod jobs;
mod pool;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::sync::Semaphore;
use tracing::info;

use crate::db::{self, DbStore, SCHEMA_VERSION};
use crate::language::{self, Language};
use crate::storage::registry;
use crate::storage::workspace::Workspace;

use http::Inner;
use jobs::JobRegistry;
use pool::ConnectionPool;

/// Entry point for `virgil-cli serve`. Synchronous setup (load
/// workspace, open the warm store, build the connection pool), then
/// blocks on the tokio/axum server until the process is signalled.
pub fn run(name: String, port: u16, max_concurrency: usize, result_ttl_secs: u64) -> Result<()> {
    if max_concurrency == 0 {
        bail!("--max-concurrency must be at least 1");
    }
    let result_ttl = Duration::from_secs(result_ttl_secs);

    let project = registry::get_project(&name)?;
    let languages = match &project.languages {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };
    let workspace = Workspace::load(&project.path, &languages, None)?;

    let cache_path = db::cache_dir_for_db(&name)?;
    if !cache_path.exists() {
        bail!(
            "project '{name}' is not parsed (no cached store at {}).\n\
             Build it first, e.g.: virgil-cli projects query {name} \
             --sql 'SELECT 1' --rebuild",
            cache_path.display()
        );
    }
    let store = DbStore::open_persistent(&cache_path)
        .with_context(|| format!("opening store at {}", cache_path.display()))?;
    if store.fresh() {
        // The file existed but was stale/incompatible and got reset to an
        // empty schema on open. Serve refuses to expose an empty store.
        bail!(
            "project '{name}' store was stale or incompatible and is now empty.\n\
             Rebuild it: virgil-cli projects query {name} --sql 'SELECT 1' --rebuild"
        );
    }

    let pool = ConnectionPool::build(&store, max_concurrency)?;
    let state = Arc::new(Inner {
        project: name,
        schema_version: SCHEMA_VERSION,
        workspace,
        pool,
        jobs: JobRegistry::new(),
        sem: Arc::new(Semaphore::new(max_concurrency)),
    });

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    rt.block_on(serve_loop(state, port, result_ttl))
}

async fn serve_loop(state: Arc<Inner>, port: u16, result_ttl: Duration) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    spawn_sweeper(state.clone(), result_ttl);
    info!(
        %addr,
        project = %state.project,
        ttl_secs = result_ttl.as_secs(),
        "serve listening (Ctrl-C to stop)"
    );
    // No graceful-shutdown signal wired: SIGINT terminates the process
    // immediately, abandoning any in-flight queries (decision: exit
    // immediately, don't drain).
    axum::serve(listener, http::router(state))
        .await
        .context("axum serve")?;
    Ok(())
}

/// Periodically evict finished jobs older than `ttl`. Sweep cadence is a
/// quarter of the TTL, clamped to [5s, 60s].
fn spawn_sweeper(state: Arc<Inner>, ttl: Duration) {
    let interval = (ttl / 4).clamp(Duration::from_secs(5), Duration::from_secs(60));
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let n = state.jobs.evict_expired(ttl);
            if n > 0 {
                tracing::debug!(evicted = n, "swept expired job results");
            }
        }
    });
}
