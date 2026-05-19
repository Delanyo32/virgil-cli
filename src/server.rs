use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::cozo::{self, CozoStore};
use crate::graph::CodeGraph;
use crate::graph::builder::{BuildOptions, GraphBuilder};
use crate::language::Language;
use crate::queries::{QueryRequest, QuerySource, run as run_query};
use crate::storage::workspace::Workspace;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

struct AppState {
    workspace: Workspace,
    /// Wrapped in RwLock so we can lazily run `ensure_resource_graph` from
    /// query handlers (e.g. the `unreleased_resources` template) on first
    /// use without re-running it on every request.
    code_graph: RwLock<CodeGraph>,
    store: CozoStore,
    build_options: BuildOptions,
}

impl AppState {
    /// Run resource-lifecycle analysis exactly once. Subsequent calls are no-ops.
    /// When the server was booted with `--no-resource-graph`, this is a no-op.
    #[allow(dead_code)]
    fn ensure_resource_graph(&self) {
        if !self.build_options.build_resource_graph {
            return;
        }
        if let Ok(mut g) = self.code_graph.write() {
            g.ensure_resource_graph(Some(&self.workspace));
        }
    }
}

/// POST /query body.
///
/// Exactly one of `cozoscript` / `template` must be set. Mutually exclusive.
#[derive(Deserialize)]
struct ServerQueryRequest {
    /// Inline Cozoscript body.
    cozoscript: Option<String>,
    /// Built-in template name.
    template: Option<String>,
    /// Parameter bindings (key → value-as-string). Integers and booleans
    /// are auto-coerced; everything else binds as a string.
    #[serde(default)]
    params: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct ReadySignal {
    ready: bool,
    port: u16,
}

pub async fn run_server(
    workspace: Workspace,
    _source_id: &str,
    host: &str,
    port: u16,
    _lang_string: Option<String>,
    languages: Vec<Language>,
    build_options: BuildOptions,
) -> Result<()> {
    let code_graph = GraphBuilder::new(&workspace, &languages)
        .with_options(build_options)
        .build()?;

    let store = CozoStore::open_in_memory()?;
    cozo::populate(&store, &code_graph, Some(&workspace))?;

    let state = Arc::new(AppState {
        workspace,
        code_graph: RwLock::new(code_graph),
        store,
        build_options,
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/query", post(handle_query))
        .with_state(state);

    let listener = TcpListener::bind(format!("{host}:{port}")).await?;
    let actual_port = listener.local_addr()?.port();

    let ready = ReadySignal {
        ready: true,
        port: actual_port,
    };
    println!("{}", serde_json::to_string(&ready)?);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn handle_query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ServerQueryRequest>,
) -> impl IntoResponse {
    let source_kind = match (&req.cozoscript, &req.template) {
        (Some(_), Some(_)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "cozoscript and template are mutually exclusive"
                })),
            );
        }
        (None, None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "exactly one of `cozoscript` or `template` is required"
                })),
            );
        }
        (Some(_), None) => 0,
        (None, Some(_)) => 1,
    };

    let params: Vec<(String, String)> = req.params.into_iter().collect();
    let result = timeout(
        REQUEST_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let graph = state
                .code_graph
                .read()
                .map_err(|_| anyhow::anyhow!("code graph lock poisoned"))?;
            let source = if source_kind == 0 {
                QuerySource::Inline(req.cozoscript.as_deref().unwrap_or(""))
            } else {
                QuerySource::Template(req.template.as_deref().unwrap_or(""))
            };
            let output = run_query(QueryRequest {
                source,
                params,
                store: &state.store,
                graph: &graph,
                workspace: &state.workspace,
            })?;
            let elapsed = start.elapsed();
            Ok::<_, anyhow::Error>(serde_json::json!({
                "query_ms": elapsed.as_millis(),
                "result": output,
            }))
        }),
    )
    .await;

    match result {
        Ok(Ok(Ok(json_val))) => (StatusCode::OK, Json(json_val)),
        Ok(Ok(Err(e))) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Query failed: {e}")})),
        ),
        Ok(Err(join_err)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Internal error: {join_err}")})),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({"error": "Request timed out after 120 seconds"})),
        ),
    }
}
