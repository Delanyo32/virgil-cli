use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use tracing::{info, info_span, warn};

use virgil_cli::cli::{Cli, Command, LogFormat, ProjectCommand};
use virgil_cli::cozo::{self, CozoStore};
use virgil_cli::language::{self, Language};
use virgil_cli::observability::{self, sampler::ResourceSampler};
use virgil_cli::queries::{self, QueryRequest, QuerySource};
use virgil_cli::server;
use virgil_cli::storage::registry;
use virgil_cli::storage::s3::S3Location;
use virgil_cli::storage::workspace::Workspace;

enum CozoSource {
    Inline(String),
    FilePath(PathBuf),
    Template(String),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_format = match cli.log_format {
        LogFormat::Compact => observability::LogFormat::Compact,
        LogFormat::Json => observability::LogFormat::Json,
    };
    observability::init(cli.verbose, cli.quiet, log_format);

    let result = dispatch(cli.command);
    if let Err(err) = &result {
        // Surface the error chain through the log pipeline before bubbling up.
        warn!(error = %err, "command failed");
    }
    result
}

fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Projects { command } => match command {
            ProjectCommand::Create {
                name,
                path,
                exclude,
                lang,
            } => {
                let entry = registry::create_project(&name, path, exclude, lang.as_deref())?;
                info!(
                    project = %entry.name,
                    path = %entry.path.display(),
                    files = entry.file_count,
                    "created project"
                );
                for (lang, count) in &entry.language_breakdown {
                    info!(language = %lang, count, "language breakdown");
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
                if let Ok(cache_path) = cozo::cache_dir_for(&name)
                    && cache_path.exists()
                {
                    if let Err(e) = std::fs::remove_dir_all(&cache_path) {
                        warn!(path = %cache_path.display(), error = %e, "failed to remove cache dir");
                    }
                }
                info!(project = %name, "deleted project");
                Ok(())
            }

            ProjectCommand::Query {
                name,
                s3,
                lang,
                exclude,
                cozoscript,
                file,
                template,
                params,
                rebuild,
                pretty,
            } => {
                let cozo_source = match (cozoscript, file, template) {
                    (Some(s), _, _) => CozoSource::Inline(s),
                    (_, Some(p), _) => CozoSource::FilePath(p),
                    (_, _, Some(t)) => CozoSource::Template(t),
                    _ => anyhow::bail!(
                        "no query provided. Use --cozoscript '<inline>', \
                         --file <path>, or --template <name>"
                    ),
                };
                run_cozo_query(
                    cozo_source,
                    params,
                    name,
                    s3,
                    lang,
                    exclude,
                    rebuild,
                    pretty,
                )
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
                    info!(uri = %s3_uri, "loading codebase from S3");
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
                    info!(path = %local_dir.display(), "loading codebase from disk");
                    let ws = Workspace::load(local_dir, &languages, None)?;
                    let canonical =
                        std::fs::canonicalize(local_dir).unwrap_or_else(|_| local_dir.clone());
                    (ws, canonical.display().to_string())
                }
                _ => unreachable!("clap enforces exactly one of --s3 / --dir"),
            };
            info!(
                files = workspace.file_count(),
                host = %host,
                port = if port == 0 { "dynamic".to_string() } else { port.to_string() },
                "starting server",
            );

            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(server::run_server(
                workspace, &source_id, &host, port, lang, languages,
            ))?;
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_cozo_query(
    source: CozoSource,
    params: Vec<(String, String)>,
    name: Option<String>,
    s3: Option<String>,
    lang: Option<String>,
    exclude: Vec<String>,
    rebuild: bool,
    pretty: bool,
) -> Result<()> {
    let sampler = ResourceSampler::start(std::time::Duration::from_millis(250));

    let (workspace, project_name) = {
        let _span = info_span!("workspace.load").entered();
        if let Some(s3_uri) = s3 {
            let languages = match &lang {
                Some(f) => language::parse_language_filter(f),
                None => Language::all().to_vec(),
            };
            let loc = S3Location::parse(&s3_uri)?;
            let ws = Workspace::load_from_s3(&loc.bucket, &loc.prefix, &languages, &exclude, None)?;
            info!(files = ws.file_count(), source = %s3_uri, "workspace loaded");
            (ws, s3_uri)
        } else {
            let name = name.ok_or_else(|| anyhow::anyhow!("provide a project name or --s3"))?;
            let project = registry::get_project(&name)?;
            let languages = match &project.languages {
                Some(f) => language::parse_language_filter(f),
                None => Language::all().to_vec(),
            };
            let ws = Workspace::load(&project.path, &languages, None)?;
            info!(files = ws.file_count(), project = %name, "workspace loaded");
            (ws, name)
        }
    };

    let languages = match &lang {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };

    let start = Instant::now();
    let cache_path = cozo::cache_dir_for(&project_name)?;
    if rebuild && cache_path.exists() {
        info!(path = %cache_path.display(), "rebuild requested, wiping cache");
        std::fs::remove_dir_all(&cache_path)?;
    }
    let store = CozoStore::open_persistent(&cache_path)?;
    let cache_state = if store.fresh() {
        let _span = info_span!("cozo.cold_build").entered();
        let graph = {
            let _gs = info_span!("graph.build").entered();
            virgil_cli::graph::builder::GraphBuilder::new(&workspace, &languages).build()?
        };
        {
            let _ps = info_span!("cozo.populate").entered();
            cozo::populate(&store, &graph, Some(&workspace))?;
        }
        "cold"
    } else {
        let _span = info_span!("cozo.refresh").entered();
        let diff = cozo::workspace_diff(&store, &workspace)?;
        if diff.is_empty() {
            info!("workspace unchanged, warm reuse");
            "warm"
        } else {
            info!(
                added = diff.added.len(),
                modified = diff.modified.len(),
                removed = diff.removed.len(),
                "incremental refresh",
            );
            cozo::incremental_refresh(&store, &workspace, &languages, &diff)?;
            "incremental"
        }
    };

    let source_ref = match &source {
        CozoSource::Inline(s) => QuerySource::Inline(s.as_str()),
        CozoSource::FilePath(p) => QuerySource::File(p.as_path()),
        CozoSource::Template(t) => QuerySource::Template(t.as_str()),
    };
    let output = {
        let _qs = info_span!("query.run", cache_state = cache_state).entered();
        queries::run(QueryRequest {
            source: source_ref,
            params,
            store: &store,
            workspace: &workspace,
        })?
    };
    let elapsed = start.elapsed();
    let res = sampler.stop();
    info!(
        elapsed_ms = elapsed.as_millis() as u64,
        rss_mb = res.rss_mb,
        peak_rss_mb = res.peak_rss_mb,
        avg_cpu_pct = res.avg_cpu_pct,
        cache = cache_state,
        "query pipeline complete",
    );

    let envelope = serde_json::json!({
        "project": project_name,
        "query_ms": elapsed.as_millis(),
        "cache": cache_state,
        "result": output,
    });
    let s = if pretty {
        serde_json::to_string_pretty(&envelope)?
    } else {
        serde_json::to_string(&envelope)?
    };
    println!("{s}");
    Ok(())
}
