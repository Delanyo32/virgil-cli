use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;

use virgil_cli::audit;
use virgil_cli::cli::{
    AuditAction, Cli, Command, CommentsAction, FileAction, ProjectAction, ProjectQueryCommand,
    QualityCommand, SecurityCommand, SymbolAction,
};
use virgil_cli::complexity;
use virgil_cli::discovery;
use virgil_cli::language::{self, Language};
use virgil_cli::languages;
use virgil_cli::models::{
    CommentInfo, ComplexityInfo, FileMetadata, ImportInfo, ParseError, SecurityIssue, SymbolInfo,
};
use virgil_cli::output;
use virgil_cli::parser;
use virgil_cli::project;
use virgil_cli::query;
use virgil_cli::security;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Project { action } => match action {
            ProjectAction::Create {
                dir,
                name,
                language: lang_filter,
            } => project_create(&dir, name.as_deref(), lang_filter.as_deref()),

            ProjectAction::List => project_list(),

            ProjectAction::Delete { name } => project_delete(&name),

            ProjectAction::Query { name, command } => dispatch_project_query(&name, command),
        },
        Command::Audit { action } => match action {
            AuditAction::Create {
                dir,
                name,
                language: lang_filter,
            } => audit_create(&dir, name.as_deref(), lang_filter.as_deref()),

            AuditAction::List => audit_list(),

            AuditAction::Delete { name } => audit_delete(&name),

            AuditAction::Complexity {
                name,
                file,
                kind,
                sort,
                limit,
                threshold,
                format,
            } => dispatch_audit_complexity(
                &name,
                file.as_deref(),
                kind.as_deref(),
                &sort,
                limit,
                threshold,
                &format,
            ),

            AuditAction::Overview { name, format } => dispatch_audit_overview(&name, &format),

            AuditAction::Quality { name, command } => dispatch_audit_quality(&name, &command),

            AuditAction::Security { name, command } => dispatch_audit_security(&name, &command),
        },
    }
}

enum ParseResult {
    Success(
        FileMetadata,
        Vec<SymbolInfo>,
        Vec<ImportInfo>,
        Vec<CommentInfo>,
        Vec<ComplexityInfo>,
        Vec<SecurityIssue>,
    ),
    Error(ParseError),
}

fn run_parse(
    dir: &std::path::Path,
    output_dir: &std::path::Path,
    lang_filter: Option<&str>,
    compute_complexity: bool,
) -> Result<()> {
    let root = dir
        .canonicalize()
        .with_context(|| format!("invalid directory: {}", dir.display()))?;

    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        Language::all().to_vec()
    };

    if languages.is_empty() {
        anyhow::bail!("no valid languages specified");
    }

    let start = Instant::now();

    // Phase 1: Discover ALL files (regardless of extension)
    let all_discovered = discovery::discover_all_files(&root)?;
    eprintln!("Discovered {} files", all_discovered.len());

    if all_discovered.is_empty() {
        eprintln!("No files found. Nothing to do.");
        return Ok(());
    }

    // Phase 2: Partition into supported and unsupported
    let supported_extensions: Vec<&str> = languages
        .iter()
        .flat_map(|l| l.all_extensions())
        .copied()
        .collect();

    let (supported_files, unsupported_files): (Vec<_>, Vec<_>) =
        all_discovered.into_iter().partition(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| supported_extensions.contains(&ext))
        });

    eprintln!(
        "Supported: {}, Unsupported: {}",
        supported_files.len(),
        unsupported_files.len()
    );

    // Phase 3: Build FileMetadata for unsupported files (parallel)
    let unsupported_metadata: Vec<FileMetadata> = unsupported_files
        .par_iter()
        .map(|path| {
            let relative_path = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let extension = path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();

            let (size_bytes, line_count) = match std::fs::read_to_string(path) {
                Ok(content) => (content.len() as u64, content.lines().count() as u64),
                Err(_) => {
                    // Fall back to file size from metadata, 0 lines
                    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    (size, 0)
                }
            };

            FileMetadata {
                path: relative_path,
                name,
                extension,
                language: "unsupported".to_string(),
                size_bytes,
                line_count,
            }
        })
        .collect();

    // Pre-compile queries per language (shared across threads)
    let mut query_map = std::collections::HashMap::new();
    let mut import_query_map = std::collections::HashMap::new();
    let mut comment_query_map = std::collections::HashMap::new();
    for lang in &languages {
        query_map.insert(*lang, languages::compile_symbol_query(*lang)?);
        import_query_map.insert(*lang, languages::compile_import_query(*lang)?);
        comment_query_map.insert(*lang, languages::compile_comment_query(*lang)?);
    }
    let query_map = Arc::new(query_map);
    let import_query_map = Arc::new(import_query_map);
    let comment_query_map = Arc::new(comment_query_map);

    // Phase 4: Parse supported files and extract symbols + imports + comments (parallel)
    // Capture errors instead of dropping them
    let results: Vec<_> = supported_files
        .par_iter()
        .filter_map(|path| {
            let ext = path.extension()?.to_str()?;
            let lang = Language::from_extension(ext)?;
            let query = query_map.get(&lang)?;
            let import_query = import_query_map.get(&lang)?;
            let comment_query = comment_query_map.get(&lang)?;

            // Compute path info up front (needed for both success and error paths)
            let relative_path = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let file_ext = path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

            let mut ts_parser = match parser::create_parser(lang) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "Warning: failed to create parser for {}: {e}",
                        path.display()
                    );
                    return Some(ParseResult::Error(ParseError {
                        file_path: relative_path,
                        file_name,
                        extension: file_ext,
                        language: lang.as_str().to_string(),
                        error_type: "parser_creation".to_string(),
                        error_message: e.to_string(),
                        size_bytes,
                    }));
                }
            };

            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Warning: failed to read {}: {e}", path.display());
                    return Some(ParseResult::Error(ParseError {
                        file_path: relative_path,
                        file_name,
                        extension: file_ext,
                        language: lang.as_str().to_string(),
                        error_type: "file_read".to_string(),
                        error_message: e.to_string(),
                        size_bytes,
                    }));
                }
            };

            let (metadata, tree) = match parser::parse_file(&mut ts_parser, path, &root, lang) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {e}", path.display());
                    return Some(ParseResult::Error(ParseError {
                        file_path: relative_path,
                        file_name,
                        extension: file_ext,
                        language: lang.as_str().to_string(),
                        error_type: "parse_failure".to_string(),
                        error_message: e.to_string(),
                        size_bytes,
                    }));
                }
            };

            let syms =
                languages::extract_symbols(&tree, source.as_bytes(), query, &metadata.path, lang);
            let imps = languages::extract_imports(
                &tree,
                source.as_bytes(),
                import_query,
                &metadata.path,
                lang,
            );
            let cmts = languages::extract_comments(
                &tree,
                source.as_bytes(),
                comment_query,
                &metadata.path,
                lang,
            );

            let cplx = if compute_complexity {
                complexity::extract_complexity(&tree, source.as_bytes(), &syms, &metadata.path, lang)
            } else {
                Vec::new()
            };

            let sec = if compute_complexity {
                security::extract_security(&tree, source.as_bytes(), &metadata.path, lang)
            } else {
                Vec::new()
            };

            Some(ParseResult::Success(metadata, syms, imps, cmts, cplx, sec))
        })
        .collect();

    // Phase 5: Collect results — split successes and errors
    let mut all_files: Vec<FileMetadata> = Vec::new();
    let mut all_symbols: Vec<SymbolInfo> = Vec::new();
    let mut all_imports: Vec<ImportInfo> = Vec::new();
    let mut all_comments: Vec<CommentInfo> = Vec::new();
    let mut all_complexities: Vec<ComplexityInfo> = Vec::new();
    let mut all_security: Vec<SecurityIssue> = Vec::new();
    let mut all_errors: Vec<ParseError> = Vec::new();

    for result in results {
        match result {
            ParseResult::Success(metadata, syms, imps, cmts, cplx, sec) => {
                all_files.push(metadata);
                all_symbols.extend(syms);
                all_imports.extend(imps);
                all_comments.extend(cmts);
                all_complexities.extend(cplx);
                all_security.extend(sec);
            }
            ParseResult::Error(err) => {
                all_errors.push(err);
            }
        }
    }

    // Merge unsupported file metadata
    let supported_count = all_files.len();
    all_files.extend(unsupported_metadata);

    // Phase 6: Write parquet output
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output dir: {}", output_dir.display()))?;

    output::write_files_parquet(&all_files, output_dir)?;
    output::write_symbols_parquet(&all_symbols, output_dir)?;
    output::write_imports_parquet(&all_imports, output_dir)?;
    output::write_comments_parquet(&all_comments, output_dir)?;
    output::write_errors_parquet(&all_errors, output_dir)?;

    if compute_complexity {
        output::write_complexity_parquet(&all_complexities, output_dir)?;
        output::write_security_parquet(&all_security, output_dir)?;
    }

    let elapsed = start.elapsed();
    if compute_complexity {
        eprintln!(
            "Done: {} files ({} supported, {} unsupported), {} symbols, {} imports, {} comments, {} complexity entries, {} security issues, {} errors in {:.2}s",
            all_files.len(),
            supported_count,
            all_files.len() - supported_count,
            all_symbols.len(),
            all_imports.len(),
            all_comments.len(),
            all_complexities.len(),
            all_security.len(),
            all_errors.len(),
            elapsed.as_secs_f64()
        );
    } else {
        eprintln!(
            "Done: {} files ({} supported, {} unsupported), {} symbols, {} imports, {} comments, {} errors in {:.2}s",
            all_files.len(),
            supported_count,
            all_files.len() - supported_count,
            all_symbols.len(),
            all_imports.len(),
            all_comments.len(),
            all_errors.len(),
            elapsed.as_secs_f64()
        );
    }
    eprintln!(
        "Output: {}/{{files,symbols,imports,comments,errors}}.parquet",
        output_dir.display(),
    );

    Ok(())
}

fn project_create(
    dir: &std::path::Path,
    name_override: Option<&str>,
    lang_filter: Option<&str>,
) -> Result<()> {
    let canonical = dir
        .canonicalize()
        .with_context(|| format!("invalid directory: {}", dir.display()))?;

    let name = match name_override {
        Some(n) => n.to_string(),
        None => project::derive_project_name(&canonical)?,
    };
    project::validate_project_name(&name)?;

    let mut meta = project::load_metadata()?;
    if project::find_project(&meta, &name).is_some() {
        anyhow::bail!("project '{}' already exists", name);
    }

    let data_dir = project::project_data_dir(&name)?;
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create {}", data_dir.display()))?;

    if let Err(e) = run_parse(&canonical, &data_dir, lang_filter, false) {
        // Clean up on failure
        let _ = std::fs::remove_dir_all(&data_dir);
        return Err(e);
    }

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    meta.projects.push(project::ProjectEntry {
        name: name.clone(),
        repo_path: canonical.to_string_lossy().into_owned(),
        data_path: data_dir.to_string_lossy().into_owned(),
        created_at,
    });
    project::save_metadata(&meta)?;

    eprintln!("Project '{}' created", name);
    Ok(())
}

fn project_list() -> Result<()> {
    let meta = project::load_metadata()?;

    if meta.projects.is_empty() {
        println!("No projects registered.");
        return Ok(());
    }

    // Compute column widths
    let mut name_w = 4; // "NAME"
    let mut repo_w = 4; // "REPO"
    for p in &meta.projects {
        name_w = name_w.max(p.name.len());
        repo_w = repo_w.max(p.repo_path.len());
    }

    println!(
        "{:<nw$}  {:<rw$}  CREATED",
        "NAME",
        "REPO",
        nw = name_w,
        rw = repo_w
    );
    println!(
        "{:<nw$}  {:<rw$}  -------",
        "-".repeat(name_w),
        "-".repeat(repo_w),
        nw = name_w,
        rw = repo_w
    );

    for p in &meta.projects {
        let ts = chrono_lite(p.created_at);
        println!(
            "{:<nw$}  {:<rw$}  {ts}",
            p.name,
            p.repo_path,
            nw = name_w,
            rw = repo_w
        );
    }

    Ok(())
}

fn chrono_lite(epoch_secs: u64) -> String {
    // Simple UTC timestamp without pulling in chrono
    let s = epoch_secs;
    let days = s / 86400;
    let time_of_day = s % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Days since 1970-01-01
    let mut y = 1970i64;
    let mut remaining_days = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md {
            m = i;
            break;
        }
        remaining_days -= md;
    }

    format!(
        "{y:04}-{:02}-{:02} {:02}:{:02}Z",
        m + 1,
        remaining_days + 1,
        hours,
        minutes
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn project_delete(name: &str) -> Result<()> {
    let mut meta = project::load_metadata()?;

    let idx = meta
        .projects
        .iter()
        .position(|p| p.name == name)
        .with_context(|| format!("project '{}' not found", name))?;

    let entry = meta.projects.remove(idx);

    let data_path = PathBuf::from(&entry.data_path);
    if data_path.exists() {
        std::fs::remove_dir_all(&data_path)
            .with_context(|| format!("failed to remove {}", data_path.display()))?;
    }

    project::save_metadata(&meta)?;
    eprintln!("Project '{}' deleted", name);
    Ok(())
}

fn dispatch_project_query(name: &str, command: ProjectQueryCommand) -> Result<()> {
    let meta = project::load_metadata()?;
    let entry = project::find_project(&meta, name)
        .with_context(|| format!("project '{}' not found", name))?;

    let data_dir = PathBuf::from(&entry.data_path);

    let output = match command {
        ProjectQueryCommand::Overview { format, depth } => {
            let engine = query::db::QueryEngine::new(&data_dir)?;
            query::overview::run_overview(&engine, &format, depth)?
        }
        ProjectQueryCommand::Search {
            query: q,
            kind,
            exported,
            limit,
            offset,
            format,
        } => {
            let engine = query::db::QueryEngine::new(&data_dir)?;
            query::search::run_search(
                &engine,
                &q,
                kind.as_deref(),
                exported,
                limit,
                offset,
                &format,
            )?
        }
        ProjectQueryCommand::File { action } => match action {
            FileAction::Get { path, format } => {
                let engine = query::db::QueryEngine::new(&data_dir)?;
                query::outline::run_outline(&engine, &path, &format)?
            }
            FileAction::List {
                language,
                directory,
                limit,
                offset,
                sort,
                format,
            } => {
                let engine = query::db::QueryEngine::new(&data_dir)?;
                query::files::run_files(
                    &engine,
                    language.as_deref(),
                    directory.as_deref(),
                    limit,
                    offset,
                    &sort,
                    &format,
                )?
            }
            FileAction::Read {
                path,
                start_line,
                end_line,
            } => {
                let root = PathBuf::from(&entry.repo_path);
                query::read::run_read(&path, &root, start_line, end_line)?
            }
        },
        ProjectQueryCommand::Symbol { action } => match action {
            SymbolAction::Get { name: sym_name, format } => {
                let engine = query::db::QueryEngine::new(&data_dir)?;
                query::symbol::run_symbol_get(&engine, &sym_name, &format)?
            }
        },
        ProjectQueryCommand::Comments { action } => match action {
            CommentsAction::List {
                file,
                kind,
                documented,
                symbol,
                limit,
                format,
            } => {
                let engine = query::db::QueryEngine::new(&data_dir)?;
                query::comments::run_comments(
                    &engine,
                    file.as_deref(),
                    kind.as_deref(),
                    documented,
                    symbol.as_deref(),
                    limit,
                    &format,
                )?
            }
            CommentsAction::Search {
                query: q,
                file,
                kind,
                limit,
                format,
            } => {
                let engine = query::db::QueryEngine::new(&data_dir)?;
                query::comments::run_comments_search(
                    &engine,
                    &q,
                    file.as_deref(),
                    kind.as_deref(),
                    limit,
                    &format,
                )?
            }
        },
    };

    print!("{output}");
    Ok(())
}

// --- Audit handlers ---

fn audit_create(
    dir: &std::path::Path,
    name_override: Option<&str>,
    lang_filter: Option<&str>,
) -> Result<()> {
    let canonical = dir
        .canonicalize()
        .with_context(|| format!("invalid directory: {}", dir.display()))?;

    let name = match name_override {
        Some(n) => n.to_string(),
        None => audit::derive_audit_name(&canonical)?,
    };
    audit::validate_audit_name(&name)?;

    let mut meta = audit::load_audit_metadata()?;
    if audit::find_audit(&meta, &name).is_some() {
        anyhow::bail!("audit '{}' already exists", name);
    }

    let data_dir = audit::audit_data_dir(&name)?;
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create {}", data_dir.display()))?;

    if let Err(e) = run_parse(&canonical, &data_dir, lang_filter, true) {
        let _ = std::fs::remove_dir_all(&data_dir);
        return Err(e);
    }

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    meta.audits.push(audit::AuditEntry {
        name: name.clone(),
        repo_path: canonical.to_string_lossy().into_owned(),
        data_path: data_dir.to_string_lossy().into_owned(),
        created_at,
    });
    audit::save_audit_metadata(&meta)?;

    eprintln!("Audit '{}' created", name);
    Ok(())
}

fn audit_list() -> Result<()> {
    let meta = audit::load_audit_metadata()?;

    if meta.audits.is_empty() {
        println!("No audits registered.");
        return Ok(());
    }

    let mut name_w = 4;
    let mut repo_w = 4;
    for a in &meta.audits {
        name_w = name_w.max(a.name.len());
        repo_w = repo_w.max(a.repo_path.len());
    }

    println!(
        "{:<nw$}  {:<rw$}  CREATED",
        "NAME",
        "REPO",
        nw = name_w,
        rw = repo_w
    );
    println!(
        "{:<nw$}  {:<rw$}  -------",
        "-".repeat(name_w),
        "-".repeat(repo_w),
        nw = name_w,
        rw = repo_w
    );

    for a in &meta.audits {
        let ts = chrono_lite(a.created_at);
        println!(
            "{:<nw$}  {:<rw$}  {ts}",
            a.name,
            a.repo_path,
            nw = name_w,
            rw = repo_w
        );
    }

    Ok(())
}

fn audit_delete(name: &str) -> Result<()> {
    let mut meta = audit::load_audit_metadata()?;

    let idx = meta
        .audits
        .iter()
        .position(|a| a.name == name)
        .with_context(|| format!("audit '{}' not found", name))?;

    let entry = meta.audits.remove(idx);

    let data_path = PathBuf::from(&entry.data_path);
    if data_path.exists() {
        std::fs::remove_dir_all(&data_path)
            .with_context(|| format!("failed to remove {}", data_path.display()))?;
    }

    audit::save_audit_metadata(&meta)?;
    eprintln!("Audit '{}' deleted", name);
    Ok(())
}

fn dispatch_audit_complexity(
    name: &str,
    file: Option<&str>,
    kind: Option<&str>,
    sort: &virgil_cli::cli::ComplexitySortField,
    limit: usize,
    threshold: Option<u32>,
    format: &virgil_cli::cli::OutputFormat,
) -> Result<()> {
    let meta = audit::load_audit_metadata()?;
    let entry = audit::find_audit(&meta, name)
        .with_context(|| format!("audit '{}' not found", name))?;

    let data_dir = PathBuf::from(&entry.data_path);
    let engine = query::db::QueryEngine::new(&data_dir)?;
    let output = query::complexity::run_complexity(&engine, file, kind, sort, limit, threshold, format)?;
    print!("{output}");
    Ok(())
}

fn dispatch_audit_overview(
    name: &str,
    format: &virgil_cli::cli::OutputFormat,
) -> Result<()> {
    let meta = audit::load_audit_metadata()?;
    let entry = audit::find_audit(&meta, name)
        .with_context(|| format!("audit '{}' not found", name))?;

    let data_dir = PathBuf::from(&entry.data_path);
    let engine = query::db::QueryEngine::new(&data_dir)?;
    let output = query::quality::run_audit_overview(&engine, format)?;
    print!("{output}");
    Ok(())
}

fn dispatch_audit_security(name: &str, command: &SecurityCommand) -> Result<()> {
    let meta = audit::load_audit_metadata()?;
    let entry = audit::find_audit(&meta, name)
        .with_context(|| format!("audit '{}' not found", name))?;

    let data_dir = PathBuf::from(&entry.data_path);
    let engine = query::db::QueryEngine::new(&data_dir)?;

    let output = match command {
        SecurityCommand::UnsafeCalls {
            file,
            limit,
            format,
        } => query::security::run_unsafe_calls(&engine, file.as_deref(), *limit, format)?,
        SecurityCommand::StringRisks {
            file,
            limit,
            format,
        } => query::security::run_string_risks(&engine, file.as_deref(), *limit, format)?,
        SecurityCommand::HardcodedSecrets {
            file,
            limit,
            format,
        } => query::security::run_hardcoded_secrets(&engine, file.as_deref(), *limit, format)?,
    };
    print!("{output}");
    Ok(())
}

fn dispatch_audit_quality(name: &str, command: &QualityCommand) -> Result<()> {
    let meta = audit::load_audit_metadata()?;
    let entry = audit::find_audit(&meta, name)
        .with_context(|| format!("audit '{}' not found", name))?;

    let data_dir = PathBuf::from(&entry.data_path);
    let engine = query::db::QueryEngine::new(&data_dir)?;

    let output = match command {
        QualityCommand::DeadCode {
            file,
            kind,
            limit,
            format,
        } => query::quality::run_dead_code(
            &engine,
            file.as_deref(),
            kind.as_deref(),
            *limit,
            format,
        )?,
        QualityCommand::Coupling {
            file,
            sort,
            limit,
            cycles,
            format,
        } => query::quality::run_coupling(
            &engine,
            file.as_deref(),
            sort,
            *limit,
            *cycles,
            format,
        )?,
        QualityCommand::Duplication {
            file,
            min_group,
            limit,
            format,
        } => query::quality::run_duplication(
            &engine,
            file.as_deref(),
            *min_group,
            *limit,
            format,
        )?,
    };
    print!("{output}");
    Ok(())
}
