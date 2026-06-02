use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use tracing::{info, info_span, warn};

use virgil_cli::cli::{Cli, Command, LogFormat, ProjectCommand};
use virgil_cli::db::{self, DbStore};
use virgil_cli::language::{self, Language};
use virgil_cli::observability::{self, sampler::ResourceSampler};
use virgil_cli::queries::{self, QueryRequest, QuerySource};
use virgil_cli::storage::registry;
use virgil_cli::storage::workspace::Workspace;

enum QueryBody {
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
                    println!(
                        "Use 'virgil-cli projects create <name> --path <dir>' to register one."
                    );
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
                if let Ok(cache_path) = db::cache_dir_for_db(&name)
                    && cache_path.exists()
                    && let Err(e) = std::fs::remove_file(&cache_path)
                {
                    warn!(path = %cache_path.display(), error = %e, "failed to remove cache file");
                }
                info!(project = %name, "deleted project");
                Ok(())
            }

            ProjectCommand::Query {
                name,
                lang,
                exclude: _,
                sql,
                file,
                template,
                params,
                rebuild,
                pretty,
            } => {
                let body = match (sql, file, template) {
                    (Some(s), _, _) => QueryBody::Inline(s),
                    (_, Some(p), _) => QueryBody::FilePath(p),
                    (_, _, Some(t)) => QueryBody::Template(t),
                    _ => anyhow::bail!(
                        "no query provided. Use --sql '<inline>', \
                         --file <path>, or --template <name>"
                    ),
                };
                run_query(body, params, name, lang, rebuild, pretty)
            }
        },

        Command::Serve {
            name,
            port,
            max_concurrency,
            result_ttl_secs,
        } => virgil_cli::serve::run(name, port, max_concurrency, result_ttl_secs),
    }
}

fn run_query(
    source: QueryBody,
    params: Vec<(String, String)>,
    name: String,
    lang: Option<String>,
    rebuild: bool,
    pretty: bool,
) -> Result<()> {
    let sampler = ResourceSampler::start(std::time::Duration::from_millis(250));

    let (workspace, project_name) = {
        let _span = info_span!("workspace.load").entered();
        let project = registry::get_project(&name)?;
        let languages = match &project.languages {
            Some(f) => language::parse_language_filter(f),
            None => Language::all().to_vec(),
        };
        let ws = Workspace::load(&project.path, &languages, None)?;
        info!(files = ws.file_count(), project = %name, "workspace loaded");
        (ws, name)
    };

    let languages = match &lang {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };

    let start = Instant::now();
    let cache_path = db::cache_dir_for_db(&project_name)?;
    if rebuild && cache_path.exists() {
        info!(path = %cache_path.display(), "rebuild requested, wiping cache");
        std::fs::remove_file(&cache_path)?;
    }
    let store = DbStore::open_persistent(&cache_path)?;
    let cache_state = if store.fresh() {
        let _span = info_span!("db.cold_build").entered();
        let graph = {
            let _gs = info_span!("graph.build").entered();
            virgil_cli::graph::builder::GraphBuilder::new(&workspace, &languages).build(&store)?
        };
        {
            let _ps = info_span!("db.populate").entered();
            db::populate(&store, &graph, Some(&workspace))?;
        }
        "cold"
    } else {
        // Incremental refresh skipped on this branch (Q6 decision).
        // Warm reopen means "schema version matches"; we trust the
        // cached store is current. To force a rebuild, pass --rebuild.
        "warm"
    };

    let source_ref = match &source {
        QueryBody::Inline(s) => QuerySource::Inline(s.as_str()),
        QueryBody::FilePath(p) => QuerySource::File(p.as_path()),
        QueryBody::Template(t) => QuerySource::Template(t.as_str()),
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
