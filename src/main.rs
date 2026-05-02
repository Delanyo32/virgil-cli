use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;

use virgil_cli::audit;
use virgil_cli::cli::{Cli, Command, OutputFormat, ProjectCommand};
use virgil_cli::language::{self, Language};
use virgil_cli::server;
use virgil_cli::storage::registry;
use virgil_cli::storage::s3::S3Location;
use virgil_cli::storage::workspace::Workspace;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Projects { command } => match command {
            ProjectCommand::Create {
                name,
                path,
                exclude,
                lang,
            } => {
                let entry = registry::create_project(&name, path, exclude, lang.as_deref())?;
                eprintln!("Created project '{}'", entry.name);
                eprintln!("  Path: {}", entry.path.display());
                eprintln!("  Files: {}", entry.file_count);
                for (lang, count) in &entry.language_breakdown {
                    eprintln!("    {lang}: {count}");
                }
                Ok(())
            }

            ProjectCommand::List => {
                let projects = registry::list_projects()?;
                if projects.is_empty() {
                    println!("No projects registered.");
                    println!("Use 'virgil projects create <name> --path <dir>' to register one.");
                } else {
                    for p in &projects {
                        println!(
                            "{:<20} {:>6} files  {}",
                            p.name,
                            p.file_count,
                            p.path.display()
                        );
                    }
                }
                Ok(())
            }

            ProjectCommand::Delete { name } => {
                registry::delete_project(&name)?;
                eprintln!("Deleted project '{name}'");
                Ok(())
            }

            ProjectCommand::Query {
                name,
                s3,
                lang,
                exclude,
                q,
                file,
                out,
                pretty,
                max,
            } => {
                let query_json = match (q, file) {
                    (Some(inline), _) => inline,
                    (None, Some(path)) => std::fs::read_to_string(&path)
                        .map_err(|e| anyhow::anyhow!("failed to read query file: {e}"))?,
                    (None, None) => {
                        use std::io::IsTerminal;
                        if std::io::stdin().is_terminal() {
                            anyhow::bail!(
                                "no query provided. Use --q '{{...}}', --file <path>, or pipe JSON to stdin"
                            );
                        }
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin().read_to_string(&mut buf)?;
                        buf
                    }
                };

                let query: virgil_cli::query::lang::TsQuery = serde_json::from_str(&query_json)
                    .map_err(|e| anyhow::anyhow!("invalid query JSON: {e}"))?;

                let (workspace, project) = if let Some(s3_uri) = s3 {
                    let languages = match &lang {
                        Some(f) => language::parse_language_filter(f),
                        None => Language::all().to_vec(),
                    };
                    let loc = virgil_cli::storage::s3::S3Location::parse(&s3_uri)?;
                    let ws = Workspace::load_from_s3(
                        &loc.bucket,
                        &loc.prefix,
                        &languages,
                        &exclude,
                        None,
                    )?;
                    let entry = registry::ProjectEntry {
                        name: s3_uri.clone(),
                        path: std::path::PathBuf::from(&s3_uri),
                        exclude,
                        languages: lang,
                        file_count: ws.file_count(),
                        language_breakdown: HashMap::new(),
                        created_at: chrono::Utc::now(),
                    };
                    (ws, entry)
                } else {
                    let name =
                        name.ok_or_else(|| anyhow::anyhow!("provide a project name or --s3"))?;
                    let project = registry::get_project(&name)?;
                    let languages = match &project.languages {
                        Some(f) => language::parse_language_filter(f),
                        None => Language::all().to_vec(),
                    };
                    let ws = Workspace::load(&project.path, &languages, None)?;
                    (ws, project)
                };

                let start = Instant::now();
                let languages = match &project.languages {
                    Some(f) => language::parse_language_filter(f),
                    None => Language::all().to_vec(),
                };
                let graph = virgil_cli::graph::builder::GraphBuilder::new(&workspace, &languages)
                    .build()?;
                let output =
                    virgil_cli::query::engine::execute(&project, &query, max, &workspace, &graph)?;
                let elapsed = start.elapsed();

                let formatted = virgil_cli::query::format::format_results(
                    &output,
                    &out,
                    pretty,
                    &project.name,
                    elapsed.as_millis() as u64,
                );
                println!("{formatted}");
                Ok(())
            }
        },

        Command::Serve {
            s3,
            dir,
            host,
            port,
            lang,
            exclude,
        } => {
            let languages = match &lang {
                Some(f) => language::parse_language_filter(f),
                None => Language::all().to_vec(),
            };
            let (workspace, source_id) = match (s3.as_ref(), dir.as_ref()) {
                (Some(s3_uri), None) => {
                    let loc = S3Location::parse(s3_uri)?;
                    eprintln!("Loading codebase from {}…", s3_uri);
                    let ws = Workspace::load_from_s3(
                        &loc.bucket,
                        &loc.prefix,
                        &languages,
                        &exclude,
                        None,
                    )?;
                    (ws, s3_uri.clone())
                }
                (None, Some(local_dir)) => {
                    eprintln!("Loading codebase from {}…", local_dir.display());
                    let ws = Workspace::load(local_dir, &languages, None)?;
                    let canonical =
                        std::fs::canonicalize(local_dir).unwrap_or_else(|_| local_dir.clone());
                    (ws, canonical.display().to_string())
                }
                _ => unreachable!("clap enforces exactly one of --s3 / --dir"),
            };
            eprintln!(
                "Loaded {} files. Starting server on {}:{}…",
                workspace.file_count(),
                host,
                if port == 0 {
                    "dynamic".to_string()
                } else {
                    port.to_string()
                }
            );

            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(server::run_server(
                workspace, &source_id, &host, port, lang, languages,
            ))?;
            Ok(())
        }

        Command::Audit {
            dir,
            s3,
            language,
            category,
            pipeline,
            format,
            file,
            per_page,
            page,
        } => {
            let ws = resolve_audit_workspace(
                dir.as_deref(),
                s3.as_deref(),
                language.as_deref(),
                "virgil audit <DIR>",
            )?;

            if let Some(audit_file_path) = file {
                run_json_audit_file_ws(&ws, language.as_deref(), &audit_file_path, &format)
            } else {
                let languages: Vec<Language> = if let Some(ref filter) = language {
                    language::parse_language_filter(filter)
                } else {
                    Language::all().to_vec()
                };

                let start = Instant::now();
                let index =
                    virgil_cli::graph::builder::GraphBuilder::new(&ws, &languages).build()?;

                let pb = create_audit_progress_bar();
                let mut engine = audit::engine::AuditEngine::new()
                    .languages(languages)
                    .progress_bar(pb);

                if let Some(ref cats) = category {
                    let cat_list: Vec<String> =
                        cats.split(',').map(|s| s.trim().to_string()).collect();
                    engine = engine.categories(cat_list);
                }

                if let Some(ref pipes) = pipeline {
                    let pipe_list: Vec<String> =
                        pipes.split(',').map(|s| s.trim().to_string()).collect();
                    engine = engine.pipelines(pipe_list);
                }

                let (findings, summary) = engine.run(&ws, Some(&index))?;

                let output =
                    audit::format::format_findings(&findings, &summary, &format, page, per_page)?;
                print!("{output}");

                let elapsed = start.elapsed();
                eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

                Ok(())
            }
        }
    }
}

/// Resolve a workspace from either a local directory or S3 URI.
fn resolve_audit_workspace(
    dir: Option<&std::path::Path>,
    s3: Option<&str>,
    lang_filter: Option<&str>,
    usage_hint: &str,
) -> Result<Workspace> {
    if let Some(s3_uri) = s3 {
        let languages: Vec<Language> = if let Some(filter) = lang_filter {
            language::parse_language_filter(filter)
        } else {
            Language::all().to_vec()
        };
        let loc = virgil_cli::storage::s3::S3Location::parse(s3_uri)?;
        Workspace::load_from_s3(&loc.bucket, &loc.prefix, &languages, &[], Some(1_000_000))
    } else {
        let dir =
            dir.ok_or_else(|| anyhow::anyhow!("Directory or --s3 required. Usage: {usage_hint}"))?;
        let languages: Vec<Language> = if let Some(filter) = lang_filter {
            language::parse_language_filter(filter)
        } else {
            Language::all().to_vec()
        };
        Workspace::load(dir, &languages, Some(1_000_000))
    }
}

fn create_audit_progress_bar() -> indicatif::ProgressBar {
    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files",
        )
        .unwrap()
        .progress_chars("##-"),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

fn run_json_audit_file_ws(
    workspace: &Workspace,
    lang_filter: Option<&str>,
    audit_file_path: &std::path::Path,
    format: &OutputFormat,
) -> Result<()> {
    let content = std::fs::read_to_string(audit_file_path)
        .map_err(|e| anyhow::anyhow!("failed to read audit file {:?}: {e}", audit_file_path))?;
    let json_audit: virgil_cli::pipeline::loader::JsonAuditFile = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid audit JSON in {:?}: {e}", audit_file_path))?;

    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else if let Some(ref lang_list) = json_audit.languages {
        lang_list
            .iter()
            .flat_map(|s| language::parse_language_filter(s))
            .collect()
    } else {
        Language::all().to_vec()
    };

    let start = Instant::now();

    let graph = virgil_cli::graph::builder::GraphBuilder::new(workspace, &languages).build()?;

    eprintln!(
        "Running JSON audit pipeline '{}' ({})…",
        json_audit.pipeline, json_audit.category
    );

    let output = virgil_cli::pipeline::executor::run_pipeline(
        &json_audit.graph,
        &graph,
        Some(workspace),
        json_audit.languages.as_deref(),
        None,
        &json_audit.pipeline,
    )?;

    let findings = match output {
        virgil_cli::pipeline::executor::PipelineOutput::Findings(f) => f,
        virgil_cli::pipeline::executor::PipelineOutput::Results(results) => {
            // Pipeline did not end with a Flag stage — report as info findings
            results
                .into_iter()
                .map(|r| virgil_cli::pipeline::output::AuditFinding {
                    file_path: r.file,
                    line: r.line,
                    column: 1,
                    severity: "info".to_string(),
                    pipeline: json_audit.pipeline.clone(),
                    pattern: json_audit.pipeline.clone(),
                    message: format!("matched by pipeline '{}'", json_audit.pipeline),
                    snippet: String::new(),
                })
                .collect()
        }
    };

    // Build a minimal AuditSummary from the findings
    let total = findings.len();
    let files_with_findings: std::collections::HashSet<&str> =
        findings.iter().map(|f| f.file_path.as_str()).collect();
    let summary = virgil_cli::audit::models::AuditSummary {
        total_findings: total,
        files_scanned: workspace.file_count(),
        files_with_findings: files_with_findings.len(),
        by_pipeline: vec![(json_audit.pipeline.clone(), total)],
        by_pattern: vec![(json_audit.pipeline.clone(), total)],
        by_pipeline_pattern: vec![(
            json_audit.pipeline.clone(),
            vec![(json_audit.pipeline.clone(), total)],
        )],
    };

    let output_str = audit::format::format_findings(&findings, &summary, format, 1, 20)?;
    print!("{output_str}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}
