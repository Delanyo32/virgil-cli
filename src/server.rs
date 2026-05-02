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

use crate::audit::engine::AuditEngine;
use crate::audit::models::AuditFinding;
use crate::graph::CodeGraph;
use crate::graph::builder::GraphBuilder;
use crate::language::Language;
use crate::query::engine;
use crate::query::lang::TsQuery;
use crate::storage::registry::ProjectEntry;
use crate::storage::workspace::Workspace;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

struct AppState {
    workspace: Workspace,
    project: ProjectEntry,
    languages: Vec<Language>,
    code_graph: CodeGraph,
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
    source_id: &str,
    host: &str,
    port: u16,
    lang_string: Option<String>,
    languages: Vec<Language>,
) -> Result<()> {
    let project = ProjectEntry {
        name: source_id.to_string(),
        path: std::path::PathBuf::from(source_id),
        exclude: vec![],
        languages: lang_string,
        file_count: workspace.file_count(),
        language_breakdown: HashMap::new(),
        created_at: chrono::Utc::now(),
    };

    let code_graph = GraphBuilder::new(&workspace, &languages).build()?;

    let state = Arc::new(AppState {
        workspace,
        project,
        languages,
        code_graph,
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
            let output = engine::execute(
                &state.project,
                &query,
                max,
                &state.workspace,
                &state.code_graph,
            )?;
            let elapsed = start.elapsed();

            let formatted = crate::query::format::format_results(
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

            let categories: Vec<(&str, Vec<Language>)> = vec![
                (
                    "tech_debt",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                (
                    "complexity",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                (
                    "code_style",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                (
                    "security",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                (
                    "scalability",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                (
                    "architecture",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
            ];

            let attempted = categories
                .iter()
                .filter(|(_, langs)| !langs.is_empty())
                .count();
            let mut total_scanned = 0;
            let mut files_with_findings = std::collections::HashSet::new();
            let mut errors: Vec<serde_json::Value> = Vec::new();

            for (name, langs) in categories {
                if langs.is_empty() {
                    continue;
                }
                let engine = AuditEngine::new()
                    .languages(langs)
                    .categories(vec![name.to_string()]);
                match engine.run(&state.workspace, Some(&state.code_graph)) {
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

            let (cat, langs) = match category.as_str() {
                "architecture" => (
                    "architecture",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                "security" => (
                    "security",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                "scalability" => (
                    "scalability",
                    filter_languages(user_languages, Language::all().to_vec()),
                ),
                _ => unreachable!(),
            };

            if langs.is_empty() {
                return Ok(serde_json::json!([]));
            }

            let engine = AuditEngine::new()
                .languages(langs)
                .categories(vec![cat.to_string()]);

            match engine.run(&state.workspace, Some(&state.code_graph)) {
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

fn run_code_quality_audit_blocking(state: &AppState, per_page: usize) -> Result<serde_json::Value> {
    let user_languages = &state.languages;
    let mut all_findings: Vec<AuditFinding> = Vec::new();

    let sub_categories: Vec<(&str, Vec<Language>)> = vec![
        (
            "tech_debt",
            filter_languages(user_languages, Language::all().to_vec()),
        ),
        (
            "complexity",
            filter_languages(user_languages, Language::all().to_vec()),
        ),
        (
            "code_style",
            filter_languages(user_languages, Language::all().to_vec()),
        ),
    ];

    for (cat, langs) in sub_categories {
        if langs.is_empty() {
            continue;
        }
        let engine = AuditEngine::new()
            .languages(langs)
            .categories(vec![cat.to_string()]);
        if let Ok((findings, _)) = engine.run(&state.workspace, Some(&state.code_graph)) {
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
