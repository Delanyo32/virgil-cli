//! axum router + handlers for serve mode.
//!
//! `POST /query` mints a job and spawns its execution, returning a
//! `job_id` immediately. Execution runs on a blocking thread (DuckDB's
//! binding is synchronous) gated by a semaphore, with one pooled
//! connection checked out per job. Results stream over SSE.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Semaphore;
use tracing::{info_span, warn};

use crate::db::DbStore;
use crate::queries::runner::QueryOutput;
use crate::queries::{QueryRequest, QuerySource, run as run_query};
use crate::storage::workspace::Workspace;

use super::jobs::{JobRegistry, JobSnapshot, JobStatus};
use super::pool::ConnectionPool;

/// Shared server state. Held behind an `Arc` and cloned into each job.
pub struct Inner {
    pub project: String,
    pub schema_version: u32,
    pub workspace: Workspace,
    pub pool: ConnectionPool,
    pub jobs: JobRegistry,
    pub sem: Arc<Semaphore>,
}

pub type AppState = Arc<Inner>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/query", post(post_query))
        .route("/jobs/{id}", get(get_job).delete(cancel_job))
        .route("/jobs/{id}/events", get(job_events))
        .route("/health", get(health))
        .with_state(state)
}

#[derive(Deserialize)]
struct QueryBody {
    sql: Option<String>,
    template: Option<String>,
    #[serde(default)]
    params: HashMap<String, String>,
    /// Advisory wall-clock timeout. On expiry the job is marked
    /// `timed_out`; the query keeps running in the background (no
    /// DuckDB interrupt) and its result is discarded.
    timeout_secs: Option<u64>,
}

/// What a worker runs.
enum Body {
    Sql(String),
    Template(String),
}

struct QuerySpec {
    body: Body,
    params: Vec<(String, String)>,
}

async fn post_query(
    State(state): State<AppState>,
    Json(body): Json<QueryBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let params: Vec<(String, String)> = body.params.into_iter().collect();
    let spec = match (body.sql, body.template) {
        (Some(s), None) => QuerySpec {
            body: Body::Sql(s),
            params,
        },
        (None, Some(t)) => QuerySpec {
            body: Body::Template(t),
            params,
        },
        (Some(_), Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                "provide exactly one of `sql` or `template`, not both".into(),
            ));
        }
        (None, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                "provide one of `sql` or `template`".into(),
            ));
        }
    };

    let timeout = body.timeout_secs.map(Duration::from_secs);
    let id = state.jobs.create();
    tokio::spawn(run_job(state.clone(), id.clone(), spec, timeout));
    Ok(Json(json!({ "job_id": id })))
}

async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobSnapshot>, StatusCode> {
    state.jobs.get(&id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn cancel_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.jobs.cancel(&id) {
        Some(status) => Ok(Json(json!({ "status": status }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "project": state.project,
        "ready": true,
        "schema_version": state.schema_version,
    }))
}

fn event_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued | JobStatus::Running => "status",
        JobStatus::Done => "completed",
        JobStatus::Error => "error",
        JobStatus::Cancelled => "cancelled",
        JobStatus::TimedOut => "timed_out",
    }
}

async fn job_events(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let rx = match state.jobs.subscribe(&id) {
        Some(rx) => rx,
        None => return (StatusCode::NOT_FOUND, "unknown job").into_response(),
    };
    let stream = async_stream::stream! {
        let mut rx = rx;
        loop {
            let snap = rx.borrow_and_update().clone();
            let terminal = snap.status.is_terminal();
            match Event::default().event(event_name(snap.status)).json_data(&snap) {
                Ok(ev) => yield Ok::<Event, Infallible>(ev),
                Err(_) => break,
            }
            if terminal {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    };
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Orchestrate one job: wait for a concurrency permit, run the query on
/// a blocking thread with a pooled connection, publish the outcome.
async fn run_job(state: AppState, id: String, spec: QuerySpec, timeout: Option<Duration>) {
    let permit = match state.sem.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return, // semaphore closed (shutdown)
    };

    // Cancelled while queued → never start it (true cancel).
    if state.jobs.status(&id) == Some(JobStatus::Cancelled) {
        return;
    }
    state.jobs.mark_running(&id);

    let store = state.pool.checkout();
    let task_state = state.clone();
    let mut handle = tokio::task::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let out = execute(&store, &task_state.workspace, &spec);
        task_state.pool.checkin(store);
        (out, started.elapsed())
    });

    match timeout {
        None => {
            finalize(&state, &id, handle.await);
            drop(permit);
        }
        Some(d) => {
            tokio::select! {
                joined = &mut handle => {
                    finalize(&state, &id, joined);
                    drop(permit);
                }
                _ = tokio::time::sleep(d) => {
                    state.jobs.mark_timeout(&id);
                    // The query keeps running. Reclaim its connection +
                    // permit when it eventually finishes; the result is
                    // discarded (status is already TimedOut).
                    let state2 = state.clone();
                    let id2 = id.clone();
                    tokio::spawn(async move {
                        finalize(&state2, &id2, handle.await);
                        drop(permit);
                    });
                }
            }
        }
    }
}

type JoinOutcome = Result<(anyhow::Result<QueryOutput>, Duration), tokio::task::JoinError>;

fn finalize(state: &AppState, id: &str, joined: JoinOutcome) {
    match joined {
        Ok((Ok(output), elapsed)) => {
            let envelope = json!({
                "project": state.project,
                "query_ms": elapsed.as_millis(),
                "result": output,
            });
            state.jobs.finish_ok(id, envelope);
        }
        Ok((Err(e), _)) => state.jobs.finish_err(id, format!("{e:#}")),
        Err(join_err) => {
            warn!(job = %id, error = %join_err, "query worker panicked");
            state
                .jobs
                .finish_err(id, format!("query worker failed: {join_err}"));
        }
    }
}

fn execute(
    store: &DbStore,
    workspace: &Workspace,
    spec: &QuerySpec,
) -> anyhow::Result<QueryOutput> {
    let _span = info_span!("serve.query").entered();
    let source = match &spec.body {
        Body::Sql(s) => QuerySource::Inline(s.as_str()),
        Body::Template(t) => QuerySource::Template(t.as_str()),
    };
    run_query(QueryRequest {
        source,
        params: spec.params.clone(),
        store,
        workspace,
    })
}
