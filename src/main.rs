use std::time::Instant;

use anyhow::Result;
use clap::Parser;

use virgil_cli::audit;
use virgil_cli::cli::{
    AuditCommand, Cli, CodeQualityCommand, Command, OutputFormat, ProjectCommand,
};
use virgil_cli::language::{self, Language};
use virgil_cli::registry;

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
                q,
                file,
                out,
                pretty,
                max,
            } => {
                let project = registry::get_project(&name)?;
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

                let query: virgil_cli::query_lang::TsQuery = serde_json::from_str(&query_json)
                    .map_err(|e| anyhow::anyhow!("invalid query JSON: {e}"))?;

                let start = Instant::now();
                let output = virgil_cli::query_engine::execute(&project, &query, max)?;
                let elapsed = start.elapsed();

                let formatted = virgil_cli::format::format_results(
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

        Command::Audit {
            dir,
            language,
            format,
            command,
        } => {
            match command {
                Some(AuditCommand::CodeQuality {
                    dir,
                    language,
                    format,
                    command,
                }) => match command {
                    Some(CodeQualityCommand::TechDebt {
                        dir,
                        language: lang_filter,
                        pipeline: pipeline_filter,
                        format,
                        per_page,
                        page,
                    }) => run_tech_debt(
                        &dir,
                        lang_filter.as_deref(),
                        pipeline_filter.as_deref(),
                        &format,
                        page,
                        per_page,
                    ),
                    Some(CodeQualityCommand::Complexity {
                        dir,
                        language: lang_filter,
                        pipeline: pipeline_filter,
                        format,
                        per_page,
                        page,
                    }) => run_complexity(
                        &dir,
                        lang_filter.as_deref(),
                        pipeline_filter.as_deref(),
                        &format,
                        page,
                        per_page,
                    ),
                    Some(CodeQualityCommand::CodeStyle {
                        dir,
                        language: lang_filter,
                        pipeline: pipeline_filter,
                        format,
                        per_page,
                        page,
                    }) => run_code_style(
                        &dir,
                        lang_filter.as_deref(),
                        pipeline_filter.as_deref(),
                        &format,
                        page,
                        per_page,
                    ),
                    None => {
                        let dir = dir.ok_or_else(|| anyhow::anyhow!(
                        "Directory argument required. Usage: virgil audit code-quality <DIR>"
                    ))?;
                        run_code_quality_summary(&dir, language.as_deref(), &format)
                    }
                },
                Some(AuditCommand::Security {
                    dir,
                    language: lang_filter,
                    pipeline: pipeline_filter,
                    format,
                    per_page,
                    page,
                }) => run_security(
                    &dir,
                    lang_filter.as_deref(),
                    pipeline_filter.as_deref(),
                    &format,
                    page,
                    per_page,
                ),
                Some(AuditCommand::Scalability {
                    dir,
                    language: lang_filter,
                    pipeline: pipeline_filter,
                    format,
                    per_page,
                    page,
                }) => run_scalability(
                    &dir,
                    lang_filter.as_deref(),
                    pipeline_filter.as_deref(),
                    &format,
                    page,
                    per_page,
                ),
                Some(AuditCommand::Architecture {
                    dir,
                    language: lang_filter,
                    pipeline: pipeline_filter,
                    format,
                    per_page,
                    page,
                }) => run_architecture(
                    &dir,
                    lang_filter.as_deref(),
                    pipeline_filter.as_deref(),
                    &format,
                    page,
                    per_page,
                ),
                None => {
                    let dir = dir.ok_or_else(|| {
                        anyhow::anyhow!("Directory argument required. Usage: virgil audit <DIR>")
                    })?;
                    run_full_audit(&dir, language.as_deref(), &format)
                }
            }
        }
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

fn run_tech_debt(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    pipeline_filter: Option<&str>,
    format: &OutputFormat,
    page: usize,
    per_page: usize,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        audit::pipeline::supported_audit_languages()
    };

    let start = Instant::now();

    let mut engine = audit::engine::AuditEngine::new().languages(languages);

    if let Some(filter) = pipeline_filter {
        let names: Vec<String> = filter.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.pipelines(names);
    }

    let pb = create_audit_progress_bar();
    engine = engine.progress_bar(pb);

    let (findings, summary) = engine.run(dir)?;

    let output = audit::format::format_findings(&findings, &summary, format, page, per_page)?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn run_complexity(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    pipeline_filter: Option<&str>,
    format: &OutputFormat,
    page: usize,
    per_page: usize,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        audit::pipeline::supported_complexity_languages()
    };

    let start = Instant::now();

    let mut engine = audit::engine::AuditEngine::new()
        .languages(languages)
        .pipeline_selector(audit::engine::PipelineSelector::Complexity);

    if let Some(filter) = pipeline_filter {
        let names: Vec<String> = filter.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.pipelines(names);
    }

    let pb = create_audit_progress_bar();
    engine = engine.progress_bar(pb);

    let (findings, summary) = engine.run(dir)?;

    let output = audit::format::format_findings(&findings, &summary, format, page, per_page)?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn run_code_style(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    pipeline_filter: Option<&str>,
    format: &OutputFormat,
    page: usize,
    per_page: usize,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        audit::pipeline::supported_code_style_languages()
    };

    let start = Instant::now();

    let mut engine = audit::engine::AuditEngine::new()
        .languages(languages)
        .pipeline_selector(audit::engine::PipelineSelector::CodeStyle);

    if let Some(filter) = pipeline_filter {
        let names: Vec<String> = filter.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.pipelines(names);
    }

    let pb = create_audit_progress_bar();
    engine = engine.progress_bar(pb);

    let (findings, summary) = engine.run(dir)?;

    let output = audit::format::format_findings(&findings, &summary, format, page, per_page)?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn run_security(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    pipeline_filter: Option<&str>,
    format: &OutputFormat,
    page: usize,
    per_page: usize,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        audit::pipeline::supported_security_languages()
    };

    let start = Instant::now();

    let mut engine = audit::engine::AuditEngine::new()
        .languages(languages)
        .pipeline_selector(audit::engine::PipelineSelector::Security);

    if let Some(filter) = pipeline_filter {
        let names: Vec<String> = filter.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.pipelines(names);
    }

    let pb = create_audit_progress_bar();
    engine = engine.progress_bar(pb);

    let (findings, summary) = engine.run(dir)?;

    let output = audit::format::format_findings(&findings, &summary, format, page, per_page)?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn run_scalability(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    pipeline_filter: Option<&str>,
    format: &OutputFormat,
    page: usize,
    per_page: usize,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        audit::pipeline::supported_scalability_languages()
    };

    let start = Instant::now();

    let mut engine = audit::engine::AuditEngine::new()
        .languages(languages)
        .pipeline_selector(audit::engine::PipelineSelector::Scalability);

    if let Some(filter) = pipeline_filter {
        let names: Vec<String> = filter.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.pipelines(names);
    }

    let pb = create_audit_progress_bar();
    engine = engine.progress_bar(pb);

    let (findings, summary) = engine.run(dir)?;

    let output = audit::format::format_findings(&findings, &summary, format, page, per_page)?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn run_architecture(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    pipeline_filter: Option<&str>,
    format: &OutputFormat,
    page: usize,
    per_page: usize,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        audit::pipeline::supported_architecture_languages()
    };

    let start = Instant::now();

    let mut engine = audit::engine::AuditEngine::new()
        .languages(languages)
        .pipeline_selector(audit::engine::PipelineSelector::Architecture);

    if let Some(filter) = pipeline_filter {
        let names: Vec<String> = filter.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.pipelines(names);
    }

    let pb = create_audit_progress_bar();
    engine = engine.progress_bar(pb);

    let (findings, summary) = engine.run(dir)?;

    let output = audit::format::format_findings(&findings, &summary, format, page, per_page)?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn run_code_quality_summary(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    format: &OutputFormat,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        audit::pipeline::supported_audit_languages()
    };

    let start = Instant::now();

    let mp = indicatif::MultiProgress::new();
    let category_style =
        indicatif::ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {msg}")
            .unwrap();
    let overall = mp.add(indicatif::ProgressBar::new(3));
    overall.set_style(category_style);

    overall.set_message("Auditing: Tech Debt");
    let file_pb = mp.add(create_audit_progress_bar());
    let td_engine = audit::engine::AuditEngine::new()
        .languages(languages.clone())
        .progress_bar(file_pb);
    let (_td_findings, td_summary) = td_engine.run(dir)?;
    overall.inc(1);

    overall.set_message("Auditing: Complexity");
    let file_pb = mp.add(create_audit_progress_bar());
    let cx_languages: Vec<Language> = languages
        .clone()
        .into_iter()
        .filter(|l| audit::pipeline::supported_complexity_languages().contains(l))
        .collect();
    let cx_engine = audit::engine::AuditEngine::new()
        .languages(cx_languages)
        .pipeline_selector(audit::engine::PipelineSelector::Complexity)
        .progress_bar(file_pb);
    let (_cx_findings, cx_summary) = cx_engine.run(dir)?;
    overall.inc(1);

    overall.set_message("Auditing: Code Style");
    let file_pb = mp.add(create_audit_progress_bar());
    let cs_languages: Vec<Language> = languages
        .into_iter()
        .filter(|l| audit::pipeline::supported_code_style_languages().contains(l))
        .collect();
    let cs_engine = audit::engine::AuditEngine::new()
        .languages(cs_languages)
        .pipeline_selector(audit::engine::PipelineSelector::CodeStyle)
        .progress_bar(file_pb);
    let (_cs_findings, cs_summary) = cs_engine.run(dir)?;
    overall.inc(1);

    overall.finish_and_clear();

    let summaries = vec![
        ("Tech Debt", &td_summary),
        ("Complexity", &cx_summary),
        ("Code Style", &cs_summary),
    ];
    let output = audit::format::format_code_quality_summary(&summaries, format, None)?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn run_full_audit(
    dir: &std::path::Path,
    lang_filter: Option<&str>,
    format: &OutputFormat,
) -> Result<()> {
    let start = Instant::now();

    let mp = indicatif::MultiProgress::new();
    let category_style =
        indicatif::ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {msg}")
            .unwrap();
    let overall = mp.add(indicatif::ProgressBar::new(6));
    overall.set_style(category_style);

    // Tech Debt
    overall.set_message("Auditing: Tech Debt");
    let file_pb = mp.add(create_audit_progress_bar());
    let td_languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
            .into_iter()
            .filter(|l| audit::pipeline::supported_audit_languages().contains(l))
            .collect()
    } else {
        audit::pipeline::supported_audit_languages()
    };
    let (_, td_summary) = audit::engine::AuditEngine::new()
        .languages(td_languages)
        .progress_bar(file_pb)
        .run(dir)?;
    overall.inc(1);

    // Complexity
    overall.set_message("Auditing: Complexity");
    let file_pb = mp.add(create_audit_progress_bar());
    let cx_languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
            .into_iter()
            .filter(|l| audit::pipeline::supported_complexity_languages().contains(l))
            .collect()
    } else {
        audit::pipeline::supported_complexity_languages()
    };
    let (_, cx_summary) = audit::engine::AuditEngine::new()
        .languages(cx_languages)
        .pipeline_selector(audit::engine::PipelineSelector::Complexity)
        .progress_bar(file_pb)
        .run(dir)?;
    overall.inc(1);

    // Code Style
    overall.set_message("Auditing: Code Style");
    let file_pb = mp.add(create_audit_progress_bar());
    let cs_languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
            .into_iter()
            .filter(|l| audit::pipeline::supported_code_style_languages().contains(l))
            .collect()
    } else {
        audit::pipeline::supported_code_style_languages()
    };
    let (_, cs_summary) = audit::engine::AuditEngine::new()
        .languages(cs_languages)
        .pipeline_selector(audit::engine::PipelineSelector::CodeStyle)
        .progress_bar(file_pb)
        .run(dir)?;
    overall.inc(1);

    // Security
    overall.set_message("Auditing: Security");
    let file_pb = mp.add(create_audit_progress_bar());
    let sec_languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
            .into_iter()
            .filter(|l| audit::pipeline::supported_security_languages().contains(l))
            .collect()
    } else {
        audit::pipeline::supported_security_languages()
    };
    let (_, sec_summary) = audit::engine::AuditEngine::new()
        .languages(sec_languages)
        .pipeline_selector(audit::engine::PipelineSelector::Security)
        .progress_bar(file_pb)
        .run(dir)?;
    overall.inc(1);

    // Scalability
    overall.set_message("Auditing: Scalability");
    let file_pb = mp.add(create_audit_progress_bar());
    let scl_languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
            .into_iter()
            .filter(|l| audit::pipeline::supported_scalability_languages().contains(l))
            .collect()
    } else {
        audit::pipeline::supported_scalability_languages()
    };
    let (_, scl_summary) = audit::engine::AuditEngine::new()
        .languages(scl_languages)
        .pipeline_selector(audit::engine::PipelineSelector::Scalability)
        .progress_bar(file_pb)
        .run(dir)?;
    overall.inc(1);

    // Architecture
    overall.set_message("Auditing: Architecture");
    let file_pb = mp.add(create_audit_progress_bar());
    let arch_languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
            .into_iter()
            .filter(|l| audit::pipeline::supported_architecture_languages().contains(l))
            .collect()
    } else {
        audit::pipeline::supported_architecture_languages()
    };
    let (_, arch_summary) = audit::engine::AuditEngine::new()
        .languages(arch_languages)
        .pipeline_selector(audit::engine::PipelineSelector::Architecture)
        .progress_bar(file_pb)
        .run(dir)?;
    overall.inc(1);

    overall.finish_and_clear();

    let summaries = vec![
        ("Tech Debt", &td_summary),
        ("Complexity", &cx_summary),
        ("Code Style", &cs_summary),
        ("Security", &sec_summary),
        ("Scalability", &scl_summary),
        ("Architecture", &arch_summary),
    ];
    let output =
        audit::format::format_code_quality_summary(&summaries, format, Some("Audit Report"))?;
    print!("{output}");

    let elapsed = start.elapsed();
    eprintln!("Completed in {:.2}s", elapsed.as_secs_f64());

    Ok(())
}
