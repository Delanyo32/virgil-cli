use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::audit;
use crate::audit::engine::{AuditEngine, PipelineSelector};
use crate::audit::models::AuditFinding;
use crate::language::Language;
use crate::query_engine;
use crate::query_lang::TsQuery;
use crate::registry::ProjectEntry;
use crate::workspace::Workspace;

struct AppState {
    workspace: Workspace,
    project: ProjectEntry,
}

#[derive(Deserialize)]
struct QueryRequest {
    query: TsQuery,
    format: Option<String>,
    max: Option<usize>,
}

#[derive(Deserialize)]
struct AuditCategoryRequest {
    per_page: Option<usize>,
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
    s3_uri: &str,
    port: u16,
    languages: Option<String>,
) -> Result<()> {
    let project = ProjectEntry {
        name: s3_uri.to_string(),
        path: std::path::PathBuf::from(s3_uri),
        exclude: vec![],
        languages,
        file_count: workspace.file_count(),
        language_breakdown: HashMap::new(),
        created_at: chrono::Utc::now(),
    };

    let state = Arc::new(AppState { workspace, project });

    let app = Router::new()
        .route("/health", get(health))
        .route("/query", post(handle_query))
        .route("/audit/summary", post(handle_audit_summary))
        .route("/audit/{category}", post(handle_audit_category))
        .with_state(state);

    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
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
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let max = req.max.unwrap_or(50);
    let query = req.query;

    let start = Instant::now();
    let output = match query_engine::execute(&state.project, &query, max, &state.workspace) {
        Ok(o) => o,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid query: {e}")})),
            );
        }
    };
    let elapsed = start.elapsed();

    let format_str = req.format.as_deref().unwrap_or("outline");
    let out_format = match format_str {
        "outline" => crate::cli::QueryOutputFormat::Outline,
        "snippet" => crate::cli::QueryOutputFormat::Snippet,
        "full" => crate::cli::QueryOutputFormat::Full,
        "tree" => crate::cli::QueryOutputFormat::Tree,
        "locations" => crate::cli::QueryOutputFormat::Locations,
        "summary" => crate::cli::QueryOutputFormat::Summary,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Unknown format: {other}")})),
            );
        }
    };

    let formatted = crate::format::format_results(
        &output,
        &out_format,
        true,
        &state.project.name,
        elapsed.as_millis() as u64,
    );

    match serde_json::from_str::<serde_json::Value>(&formatted) {
        Ok(json_val) => (StatusCode::OK, Json(json_val)),
        Err(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"results": formatted})),
        ),
    }
}

async fn handle_audit_summary(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let all_languages = Language::all().to_vec();

    let categories: Vec<(PipelineSelector, Vec<Language>)> = vec![
        (
            PipelineSelector::TechDebt,
            filter_languages(&all_languages, audit::pipeline::supported_audit_languages()),
        ),
        (
            PipelineSelector::Complexity,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_complexity_languages(),
            ),
        ),
        (
            PipelineSelector::CodeStyle,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_code_style_languages(),
            ),
        ),
        (
            PipelineSelector::Security,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_security_languages(),
            ),
        ),
        (
            PipelineSelector::Scalability,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_scalability_languages(),
            ),
        ),
        (
            PipelineSelector::Architecture,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_architecture_languages(),
            ),
        ),
    ];

    let mut total_scanned = 0;
    let mut files_with_findings = std::collections::HashSet::new();

    for (selector, langs) in categories {
        if langs.is_empty() {
            continue;
        }
        let engine = AuditEngine::new()
            .languages(langs)
            .pipeline_selector(selector);
        if let Ok((findings, summary)) = engine.run(&state.workspace) {
            total_scanned = total_scanned.max(summary.files_scanned);
            for f in &findings {
                files_with_findings.insert(f.file_path.clone());
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "files_scanned": total_scanned,
            "files_with_findings": files_with_findings.len()
        })),
    )
}

async fn handle_audit_category(
    State(state): State<Arc<AppState>>,
    Path(category): Path<String>,
    body: Option<Json<AuditCategoryRequest>>,
) -> impl IntoResponse {
    let per_page = body.and_then(|b| b.per_page).unwrap_or(100_000);
    let all_languages = Language::all().to_vec();

    let (selector, langs) = match category.as_str() {
        "architecture" => (
            PipelineSelector::Architecture,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_architecture_languages(),
            ),
        ),
        "security" => (
            PipelineSelector::Security,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_security_languages(),
            ),
        ),
        "scalability" => (
            PipelineSelector::Scalability,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_scalability_languages(),
            ),
        ),
        "code-quality" => {
            return run_code_quality_audit(&state, per_page);
        }
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": format!("Unknown category: {other}. Valid: architecture, security, scalability, code-quality")}),
                ),
            );
        }
    };

    if langs.is_empty() {
        return (StatusCode::OK, Json(serde_json::json!([])));
    }

    let engine = AuditEngine::new()
        .languages(langs)
        .pipeline_selector(selector);

    match engine.run(&state.workspace) {
        Ok((findings, _)) => {
            let limited: Vec<&AuditFinding> = findings.iter().take(per_page).collect();
            (
                StatusCode::OK,
                Json(serde_json::to_value(&limited).unwrap_or_default()),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Audit failed: {e}")})),
        ),
    }
}

fn run_code_quality_audit(
    state: &AppState,
    per_page: usize,
) -> (StatusCode, Json<serde_json::Value>) {
    let all_languages = Language::all().to_vec();
    let mut all_findings: Vec<AuditFinding> = Vec::new();

    let sub_categories = vec![
        (
            PipelineSelector::TechDebt,
            filter_languages(&all_languages, audit::pipeline::supported_audit_languages()),
        ),
        (
            PipelineSelector::Complexity,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_complexity_languages(),
            ),
        ),
        (
            PipelineSelector::CodeStyle,
            filter_languages(
                &all_languages,
                audit::pipeline::supported_code_style_languages(),
            ),
        ),
    ];

    for (selector, langs) in sub_categories {
        if langs.is_empty() {
            continue;
        }
        let engine = AuditEngine::new()
            .languages(langs)
            .pipeline_selector(selector);
        if let Ok((findings, _)) = engine.run(&state.workspace) {
            all_findings.extend(findings);
        }
    }

    let limited: Vec<&AuditFinding> = all_findings.iter().take(per_page).collect();
    (
        StatusCode::OK,
        Json(serde_json::to_value(&limited).unwrap_or_default()),
    )
}

fn filter_languages(available: &[Language], supported: Vec<Language>) -> Vec<Language> {
    available
        .iter()
        .copied()
        .filter(|l| supported.contains(l))
        .collect()
}
