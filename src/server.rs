use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::audit;
use crate::audit::engine::{AuditEngine, PipelineSelector};
use crate::audit::models::AuditFinding;
use crate::audit::project_index::ProjectIndex;
use crate::language::Language;
use crate::query_engine;
use crate::query_lang::TsQuery;
use crate::registry::ProjectEntry;
use crate::workspace::Workspace;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

struct AppState {
    workspace: Workspace,
    project: ProjectEntry,
    languages: Vec<Language>,
    project_index: ProjectIndex,
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
    host: &str,
    port: u16,
    lang_string: Option<String>,
    languages: Vec<Language>,
) -> Result<()> {
    let project = ProjectEntry {
        name: s3_uri.to_string(),
        path: std::path::PathBuf::from(s3_uri),
        exclude: vec![],
        languages: lang_string,
        file_count: workspace.file_count(),
        language_breakdown: HashMap::new(),
        created_at: chrono::Utc::now(),
    };

    let project_index = audit::index_builder::build_index(&workspace, &languages)?;

    let state = Arc::new(AppState {
        workspace,
        project,
        languages,
        project_index,
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/query", post(handle_query))
        .route("/audit/summary", post(handle_audit_summary))
        .route("/audit/{category}", post(handle_audit_category))
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
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let max = req.max.unwrap_or(50);
    let format_str = req.format.unwrap_or_else(|| "outline".to_string());

    let out_format = match format_str.as_str() {
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

    let state = Arc::clone(&state);
    let query = req.query;

    let result = timeout(
        REQUEST_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let output =
                query_engine::execute(&state.project, &query, max, &state.workspace)?;
            let elapsed = start.elapsed();

            let formatted = crate::format::format_results(
                &output,
                &out_format,
                true,
                &state.project.name,
                elapsed.as_millis() as u64,
            );
            Ok::<_, anyhow::Error>(formatted)
        }),
    )
    .await;

    match result {
        Ok(Ok(Ok(formatted))) => match serde_json::from_str::<serde_json::Value>(&formatted) {
            Ok(json_val) => (StatusCode::OK, Json(json_val)),
            Err(_) => (
                StatusCode::OK,
                Json(serde_json::json!({"results": formatted})),
            ),
        },
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

async fn handle_audit_summary(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let state = Arc::clone(&state);

    let result = timeout(
        REQUEST_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let user_languages = &state.languages;

            let categories: Vec<(&str, PipelineSelector, Vec<Language>)> = vec![
                (
                    "tech_debt",
                    PipelineSelector::TechDebt,
                    filter_languages(user_languages, audit::pipeline::supported_audit_languages()),
                ),
                (
                    "complexity",
                    PipelineSelector::Complexity,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_complexity_languages(),
                    ),
                ),
                (
                    "code_style",
                    PipelineSelector::CodeStyle,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_code_style_languages(),
                    ),
                ),
                (
                    "security",
                    PipelineSelector::Security,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_security_languages(),
                    ),
                ),
                (
                    "scalability",
                    PipelineSelector::Scalability,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_scalability_languages(),
                    ),
                ),
                (
                    "architecture",
                    PipelineSelector::Architecture,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_architecture_languages(),
                    ),
                ),
            ];

            let attempted = categories.iter().filter(|(_, _, langs)| !langs.is_empty()).count();
            let mut total_scanned = 0;
            let mut files_with_findings = std::collections::HashSet::new();
            let mut errors: Vec<serde_json::Value> = Vec::new();

            for (name, selector, langs) in categories {
                if langs.is_empty() {
                    continue;
                }
                let index_ref = match selector {
                    PipelineSelector::Architecture | PipelineSelector::CodeStyle => {
                        Some(&state.project_index)
                    }
                    _ => None,
                };
                let engine = AuditEngine::new()
                    .languages(langs)
                    .pipeline_selector(selector);
                match engine.run(&state.workspace, index_ref) {
                    Ok((findings, summary)) => {
                        total_scanned = total_scanned.max(summary.files_scanned);
                        for f in &findings {
                            files_with_findings.insert(f.file_path.clone());
                        }
                    }
                    Err(e) => {
                        errors.push(serde_json::json!({
                            "category": name,
                            "error": format!("{e}")
                        }));
                    }
                }
            }

            (total_scanned, files_with_findings.len(), errors, attempted)
        }),
    )
    .await;

    match result {
        Ok(Ok((total_scanned, findings_count, errors, attempted))) => {
            if !errors.is_empty() && errors.len() >= attempted {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "All audit categories failed",
                        "details": errors
                    })),
                )
            } else {
                let mut response = serde_json::json!({
                    "files_scanned": total_scanned,
                    "files_with_findings": findings_count
                });
                if !errors.is_empty() {
                    response["errors"] = serde_json::json!(errors);
                }
                (StatusCode::OK, Json(response))
            }
        }
        Ok(Err(join_err)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Internal error: {join_err}")})),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({"error": "Audit summary timed out after 120 seconds"})),
        ),
    }
}

async fn handle_audit_category(
    State(state): State<Arc<AppState>>,
    Path(category): Path<String>,
    body: Option<Json<AuditCategoryRequest>>,
) -> impl IntoResponse {
    let per_page = body.and_then(|b| b.per_page).unwrap_or(100_000);

    if !matches!(
        category.as_str(),
        "architecture" | "security" | "scalability" | "code-quality"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "Unknown category: {}. Valid: architecture, security, scalability, code-quality",
                    category
                )
            })),
        );
    }

    let state = Arc::clone(&state);

    let result = timeout(
        REQUEST_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let user_languages = &state.languages;

            if category == "code-quality" {
                return run_code_quality_audit_blocking(&state, per_page);
            }

            let (selector, langs) = match category.as_str() {
                "architecture" => (
                    PipelineSelector::Architecture,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_architecture_languages(),
                    ),
                ),
                "security" => (
                    PipelineSelector::Security,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_security_languages(),
                    ),
                ),
                "scalability" => (
                    PipelineSelector::Scalability,
                    filter_languages(
                        user_languages,
                        audit::pipeline::supported_scalability_languages(),
                    ),
                ),
                _ => unreachable!(),
            };

            if langs.is_empty() {
                return Ok(serde_json::json!([]));
            }

            let index_ref = match selector {
                PipelineSelector::Architecture => Some(&state.project_index),
                _ => None,
            };
            let engine = AuditEngine::new()
                .languages(langs)
                .pipeline_selector(selector);

            match engine.run(&state.workspace, index_ref) {
                Ok((findings, _)) => {
                    let limited: Vec<&AuditFinding> = findings.iter().take(per_page).collect();
                    Ok(serde_json::to_value(&limited).unwrap_or_default())
                }
                Err(e) => Err(e),
            }
        }),
    )
    .await;

    match result {
        Ok(Ok(Ok(json_val))) => (StatusCode::OK, Json(json_val)),
        Ok(Ok(Err(e))) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Audit failed: {e}")})),
        ),
        Ok(Err(join_err)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Internal error: {join_err}")})),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({"error": "Audit timed out after 120 seconds"})),
        ),
    }
}

fn run_code_quality_audit_blocking(
    state: &AppState,
    per_page: usize,
) -> Result<serde_json::Value> {
    let user_languages = &state.languages;
    let mut all_findings: Vec<AuditFinding> = Vec::new();

    let sub_categories: Vec<(&str, PipelineSelector, Vec<Language>)> = vec![
        (
            "tech_debt",
            PipelineSelector::TechDebt,
            filter_languages(user_languages, audit::pipeline::supported_audit_languages()),
        ),
        (
            "complexity",
            PipelineSelector::Complexity,
            filter_languages(
                user_languages,
                audit::pipeline::supported_complexity_languages(),
            ),
        ),
        (
            "code_style",
            PipelineSelector::CodeStyle,
            filter_languages(
                user_languages,
                audit::pipeline::supported_code_style_languages(),
            ),
        ),
    ];

    for (_, selector, langs) in sub_categories {
        if langs.is_empty() {
            continue;
        }
        let index_ref = match selector {
            PipelineSelector::CodeStyle => Some(&state.project_index),
            _ => None,
        };
        let engine = AuditEngine::new()
            .languages(langs)
            .pipeline_selector(selector);
        if let Ok((findings, _)) = engine.run(&state.workspace, index_ref) {
            all_findings.extend(findings);
        }
    }

    let limited: Vec<&AuditFinding> = all_findings.iter().take(per_page).collect();
    Ok(serde_json::to_value(&limited).unwrap_or_default())
}

fn filter_languages(available: &[Language], supported: Vec<Language>) -> Vec<Language> {
    available
        .iter()
        .copied()
        .filter(|l| supported.contains(l))
        .collect()
}
