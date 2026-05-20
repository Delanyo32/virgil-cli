use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;

use virgil_cli::cli::{Cli, Command, ProjectCommand};
use virgil_cli::cozo::{self, CozoStore};
use virgil_cli::language::{self, Language};
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
                if let Ok(cache_path) = cozo::cache_dir_for(&name)
                    && cache_path.exists()
                {
                    let _ = std::fs::remove_dir_all(&cache_path);
                }
                eprintln!("Deleted project '{name}'");
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
                workspace,
                &source_id,
                &host,
                port,
                lang,
                languages,
            ))?;
            Ok(())
        }

    }
}

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
    let (workspace, project_name) = if let Some(s3_uri) = s3 {
        let languages = match &lang {
            Some(f) => language::parse_language_filter(f),
            None => Language::all().to_vec(),
        };
        let loc = S3Location::parse(&s3_uri)?;
        let ws = Workspace::load_from_s3(&loc.bucket, &loc.prefix, &languages, &exclude, None)?;
        (ws, s3_uri)
    } else {
        let name = name.ok_or_else(|| anyhow::anyhow!("provide a project name or --s3"))?;
        let project = registry::get_project(&name)?;
        let languages = match &project.languages {
            Some(f) => language::parse_language_filter(f),
            None => Language::all().to_vec(),
        };
        let ws = Workspace::load(&project.path, &languages, None)?;
        (ws, name)
    };

    let languages = match &lang {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };

    let start = Instant::now();
    let cache_path = cozo::cache_dir_for(&project_name)?;
    if rebuild && cache_path.exists() {
        std::fs::remove_dir_all(&cache_path)?;
    }
    let store = CozoStore::open_persistent(&cache_path)?;
    let cache_state = if store.fresh() {
        // Cold start — full build + populate.
        let graph =
            virgil_cli::graph::builder::GraphBuilder::new(&workspace, &languages).build()?;
        cozo::populate(&store, &graph, Some(&workspace))?;
        cozo::resolve_cross_file_edges(&store)?;
        "cold"
    } else {
        let diff = cozo::workspace_diff(&store, &workspace)?;
        if diff.is_empty() {
            "warm"
        } else {
            cozo::incremental_refresh(&store, &workspace, &languages, &diff)?;
            "incremental"
        }
    };

    let source_ref = match &source {
        CozoSource::Inline(s) => QuerySource::Inline(s.as_str()),
        CozoSource::FilePath(p) => QuerySource::File(p.as_path()),
        CozoSource::Template(t) => QuerySource::Template(t.as_str()),
    };
    let output = queries::run(QueryRequest {
        source: source_ref,
        params,
        store: &store,
        workspace: &workspace,
    })?;
    let elapsed = start.elapsed();

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
