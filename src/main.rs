use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;

use virgil_cli::cli::{
    Cli, Command, CommentsAction, FileAction, OutputFormat, ProjectAction, ProjectQueryCommand,
    SymbolAction,
};
use virgil_cli::discovery;
use virgil_cli::language::{self, Language};
use virgil_cli::languages;
use virgil_cli::models::{CommentInfo, FileMetadata, ImportInfo, ParseError, SymbolInfo};
use virgil_cli::output;
use virgil_cli::parser;
use virgil_cli::project;
use virgil_cli::query;
use virgil_cli::s3;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let s3_config = if cli.env {
        Some(s3::S3Config::from_env()?)
    } else {
        None
    };

    dispatch_command(cli.command, s3_config)
}

fn dispatch_command(command: Command, s3_config: Option<s3::S3Config>) -> Result<()> {
    match command {
        Command::Parse {
            dir,
            output: output_dir,
            language: lang_filter,
        } => {
            if let Some(cfg) = &s3_config {
                run_parse_s3(
                    cfg,
                    &dir.to_string_lossy(),
                    &output_dir.to_string_lossy(),
                    lang_filter.as_deref(),
                )
            } else {
                run_parse(&dir, &output_dir, lang_filter.as_deref())
            }
        }

        Command::Overview {
            data_dir,
            format,
            depth,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::overview::run_overview(&engine, &format, depth)?;
            print!("{output}");
            Ok(())
        }

        Command::Search {
            query: q,
            data_dir,
            kind,
            exported,
            limit,
            offset,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::search::run_search(
                &engine,
                &q,
                kind.as_deref(),
                exported,
                limit,
                offset,
                &format,
            )?;
            print!("{output}");
            Ok(())
        }

        Command::Outline {
            file_path,
            data_dir,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::outline::run_outline(&engine, &file_path, &format)?;
            print!("{output}");
            Ok(())
        }

        Command::Files {
            data_dir,
            language: lang,
            directory,
            limit,
            offset,
            sort,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::files::run_files(
                &engine,
                lang.as_deref(),
                directory.as_deref(),
                limit,
                offset,
                &sort,
                &format,
            )?;
            print!("{output}");
            Ok(())
        }

        Command::Read {
            file_path,
            data_dir: _,
            root,
            start_line,
            end_line,
        } => {
            let output = if let Some(cfg) = &s3_config {
                let client = s3::S3Client::new(cfg)?;
                query::read::run_read_s3(
                    &file_path,
                    &root.to_string_lossy(),
                    &client,
                    start_line,
                    end_line,
                )?
            } else {
                query::read::run_read(&file_path, &root, start_line, end_line)?
            };
            print!("{output}");
            Ok(())
        }

        Command::Query {
            sql,
            data_dir,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = run_raw_query(&engine, &sql, &format)?;
            print!("{output}");
            Ok(())
        }

        Command::Deps {
            file_path,
            data_dir,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::deps::run_deps(&engine, &file_path, &format)?;
            print!("{output}");
            Ok(())
        }

        Command::Dependents {
            file_path,
            data_dir,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::dependents::run_dependents(&engine, &file_path, &format)?;
            print!("{output}");
            Ok(())
        }

        Command::Callers {
            symbol_name,
            data_dir,
            limit,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::callers::run_callers(&engine, &symbol_name, limit, &format)?;
            print!("{output}");
            Ok(())
        }

        Command::Imports {
            data_dir,
            module,
            kind,
            file,
            type_only,
            external,
            internal,
            limit,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::imports::run_imports(
                &engine,
                module.as_deref(),
                kind.as_deref(),
                file.as_deref(),
                type_only,
                external,
                internal,
                limit,
                &format,
            )?;
            print!("{output}");
            Ok(())
        }

        Command::Comments {
            data_dir,
            file,
            kind,
            documented,
            symbol,
            limit,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::comments::run_comments(
                &engine,
                file.as_deref(),
                kind.as_deref(),
                documented,
                symbol.as_deref(),
                limit,
                &format,
            )?;
            print!("{output}");
            Ok(())
        }

        Command::Errors {
            data_dir,
            error_type,
            language: lang,
            limit,
            format,
        } => {
            let engine = make_engine(&s3_config, &data_dir)?;
            let output = query::errors::run_errors(
                &engine,
                error_type.as_deref(),
                lang.as_deref(),
                limit,
                &format,
            )?;
            print!("{output}");
            Ok(())
        }

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
    }
}

fn make_engine(
    s3_config: &Option<s3::S3Config>,
    data_dir: &std::path::Path,
) -> Result<query::db::QueryEngine> {
    if let Some(cfg) = s3_config {
        query::db::QueryEngine::new_s3(cfg, &data_dir.to_string_lossy())
    } else {
        query::db::QueryEngine::new(data_dir)
    }
}

enum ParseResult {
    Success(
        FileMetadata,
        Vec<SymbolInfo>,
        Vec<ImportInfo>,
        Vec<CommentInfo>,
    ),
    Error(ParseError),
}

fn run_parse(
    dir: &std::path::Path,
    output_dir: &std::path::Path,
    lang_filter: Option<&str>,
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

            Some(ParseResult::Success(metadata, syms, imps, cmts))
        })
        .collect();

    // Phase 5: Collect results — split successes and errors
    let mut all_files: Vec<FileMetadata> = Vec::new();
    let mut all_symbols: Vec<SymbolInfo> = Vec::new();
    let mut all_imports: Vec<ImportInfo> = Vec::new();
    let mut all_comments: Vec<CommentInfo> = Vec::new();
    let mut all_errors: Vec<ParseError> = Vec::new();

    for result in results {
        match result {
            ParseResult::Success(metadata, syms, imps, cmts) => {
                all_files.push(metadata);
                all_symbols.extend(syms);
                all_imports.extend(imps);
                all_comments.extend(cmts);
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

    let elapsed = start.elapsed();
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
    eprintln!(
        "Output: {}/{{files,symbols,imports,comments,errors}}.parquet",
        output_dir.display(),
    );

    Ok(())
}

fn run_parse_s3(
    s3_config: &s3::S3Config,
    dir_prefix: &str,
    output_prefix: &str,
    lang_filter: Option<&str>,
) -> Result<()> {
    let languages: Vec<Language> = if let Some(filter) = lang_filter {
        language::parse_language_filter(filter)
    } else {
        Language::all().to_vec()
    };

    if languages.is_empty() {
        anyhow::bail!("no valid languages specified");
    }

    let start = Instant::now();

    let client = s3::S3Client::new(s3_config)?;

    // Phase 1: List ALL files from S3 prefix
    let all_files_s3 = client.list_files(dir_prefix, &[])?;
    eprintln!("Discovered {} files from S3", all_files_s3.len());

    if all_files_s3.is_empty() {
        eprintln!("No files found. Nothing to do.");
        return Ok(());
    }

    // Phase 2: Partition into supported and unsupported
    let supported_extensions: Vec<&str> = languages
        .iter()
        .flat_map(|l| l.all_extensions())
        .copied()
        .collect();

    let prefix_for_relative = dir_prefix.trim_end_matches('/');

    let (supported_s3, unsupported_s3): (Vec<_>, Vec<_>) =
        all_files_s3.into_iter().partition(|f| {
            f.key
                .rsplit('.')
                .next()
                .is_some_and(|ext| supported_extensions.contains(&ext))
        });

    eprintln!(
        "Supported: {}, Unsupported: {}",
        supported_s3.len(),
        unsupported_s3.len()
    );

    // Phase 3: Build FileMetadata for unsupported files (use size from listing, no download)
    let unsupported_metadata: Vec<FileMetadata> = unsupported_s3
        .iter()
        .map(|f| {
            let relative_path = f
                .key
                .strip_prefix(prefix_for_relative)
                .unwrap_or(&f.key)
                .trim_start_matches('/')
                .to_string();

            let name = relative_path
                .rsplit('/')
                .next()
                .unwrap_or(&relative_path)
                .to_string();

            let extension = name.rsplit('.').next().unwrap_or("").to_string();

            FileMetadata {
                path: relative_path,
                name,
                extension,
                language: "unsupported".to_string(),
                size_bytes: f.size,
                line_count: 0,
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
    let s3_config_arc = Arc::new(s3_config.clone());

    // Phase 4: Parse supported files (parallel — each task creates its own S3Client)
    let results: Vec<_> = supported_s3
        .par_iter()
        .filter_map(|s3_file| {
            let ext = s3_file.key.rsplit('.').next()?;
            let lang = Language::from_extension(ext)?;
            let query = query_map.get(&lang)?;
            let import_query = import_query_map.get(&lang)?;
            let comment_query = comment_query_map.get(&lang)?;

            let relative_path = s3_file
                .key
                .strip_prefix(prefix_for_relative)
                .unwrap_or(&s3_file.key)
                .trim_start_matches('/')
                .to_string();

            let file_name = relative_path
                .rsplit('/')
                .next()
                .unwrap_or(&relative_path)
                .to_string();

            let file_ext = file_name.rsplit('.').next().unwrap_or("").to_string();

            // Each rayon task creates its own S3Client
            let task_client = match s3::S3Client::new(&s3_config_arc) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "Warning: failed to create S3 client for {}: {e}",
                        s3_file.key
                    );
                    return Some(ParseResult::Error(ParseError {
                        file_path: relative_path,
                        file_name,
                        extension: file_ext,
                        language: lang.as_str().to_string(),
                        error_type: "file_read".to_string(),
                        error_message: e.to_string(),
                        size_bytes: s3_file.size,
                    }));
                }
            };

            let source = match task_client.get_file_string(&s3_file.key) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Warning: failed to download {}: {e}", s3_file.key);
                    return Some(ParseResult::Error(ParseError {
                        file_path: relative_path,
                        file_name,
                        extension: file_ext,
                        language: lang.as_str().to_string(),
                        error_type: "file_read".to_string(),
                        error_message: e.to_string(),
                        size_bytes: s3_file.size,
                    }));
                }
            };

            let mut ts_parser = match parser::create_parser(lang) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Warning: failed to create parser for {}: {e}", s3_file.key);
                    return Some(ParseResult::Error(ParseError {
                        file_path: relative_path,
                        file_name,
                        extension: file_ext,
                        language: lang.as_str().to_string(),
                        error_type: "parser_creation".to_string(),
                        error_message: e.to_string(),
                        size_bytes: s3_file.size,
                    }));
                }
            };

            let (metadata, tree) =
                match parser::parse_content(&mut ts_parser, &source, &relative_path, lang) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("Warning: failed to parse {}: {e}", s3_file.key);
                        return Some(ParseResult::Error(ParseError {
                            file_path: relative_path,
                            file_name,
                            extension: file_ext,
                            language: lang.as_str().to_string(),
                            error_type: "parse_failure".to_string(),
                            error_message: e.to_string(),
                            size_bytes: s3_file.size,
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

            Some(ParseResult::Success(metadata, syms, imps, cmts))
        })
        .collect();

    // Phase 5: Collect results
    let mut all_files: Vec<FileMetadata> = Vec::new();
    let mut all_symbols: Vec<SymbolInfo> = Vec::new();
    let mut all_imports: Vec<ImportInfo> = Vec::new();
    let mut all_comments: Vec<CommentInfo> = Vec::new();
    let mut all_errors: Vec<ParseError> = Vec::new();

    for result in results {
        match result {
            ParseResult::Success(metadata, syms, imps, cmts) => {
                all_files.push(metadata);
                all_symbols.extend(syms);
                all_imports.extend(imps);
                all_comments.extend(cmts);
            }
            ParseResult::Error(err) => {
                all_errors.push(err);
            }
        }
    }

    let supported_count = all_files.len();
    all_files.extend(unsupported_metadata);

    // Phase 6: Write parquet to S3
    output::write_files_parquet_s3(&all_files, &client, output_prefix)?;
    output::write_symbols_parquet_s3(&all_symbols, &client, output_prefix)?;
    output::write_imports_parquet_s3(&all_imports, &client, output_prefix)?;
    output::write_comments_parquet_s3(&all_comments, &client, output_prefix)?;
    output::write_errors_parquet_s3(&all_errors, &client, output_prefix)?;

    let elapsed = start.elapsed();
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
    eprintln!(
        "Output: s3://{}/{}/{{files,symbols,imports,comments,errors}}.parquet",
        s3_config.bucket_name,
        output_prefix.trim_end_matches('/'),
    );

    Ok(())
}

fn run_raw_query(
    engine: &query::db::QueryEngine,
    sql: &str,
    format: &OutputFormat,
) -> Result<String> {
    use duckdb::types::ValueRef;
    use serde_json::Value;

    let mut stmt = engine
        .conn
        .prepare(sql)
        .context("failed to prepare SQL query")?;

    let mut rows = stmt.query([]).context("failed to execute query")?;

    // Get column info after execution
    let column_count = rows.as_ref().unwrap().column_count();
    let column_names: Vec<String> = (0..column_count)
        .map(|i| {
            rows.as_ref()
                .unwrap()
                .column_name(i)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "?".to_string())
        })
        .collect();

    let mut collected: Vec<serde_json::Map<String, Value>> = Vec::new();
    while let Some(row) = rows.next().context("failed to fetch row")? {
        let mut map = serde_json::Map::new();
        for (i, name) in column_names.iter().enumerate() {
            let value: Value = match row.get_ref(i) {
                Ok(ValueRef::Null) => Value::Null,
                Ok(ValueRef::Boolean(b)) => Value::Bool(b),
                Ok(ValueRef::TinyInt(v)) => Value::Number(v.into()),
                Ok(ValueRef::SmallInt(v)) => Value::Number(v.into()),
                Ok(ValueRef::Int(v)) => Value::Number(v.into()),
                Ok(ValueRef::BigInt(v)) => Value::Number(v.into()),
                Ok(ValueRef::HugeInt(v)) => Value::Number((v as i64).into()),
                Ok(ValueRef::Float(v)) => serde_json::Number::from_f64(v as f64)
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
                Ok(ValueRef::Double(v)) => serde_json::Number::from_f64(v)
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
                Ok(ValueRef::Text(bytes)) => {
                    Value::String(String::from_utf8_lossy(bytes).to_string())
                }
                Ok(ValueRef::Blob(bytes)) => Value::String(format!("<blob {} bytes>", bytes.len())),
                _ => Value::Null,
            };
            map.insert(name.clone(), value);
        }
        collected.push(map);
    }

    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&collected)?),
        OutputFormat::Csv => {
            let mut out = String::new();
            out.push_str(&column_names.join(","));
            out.push('\n');
            for row in &collected {
                let cells: Vec<String> = column_names
                    .iter()
                    .map(|name| match row.get(name) {
                        Some(Value::String(s)) => {
                            if s.contains(',') || s.contains('"') {
                                format!("\"{}\"", s.replace('"', "\"\""))
                            } else {
                                s.clone()
                            }
                        }
                        Some(Value::Null) | None => String::new(),
                        Some(v) => v.to_string(),
                    })
                    .collect();
                out.push_str(&cells.join(","));
                out.push('\n');
            }
            Ok(out)
        }
        OutputFormat::Table => {
            if collected.is_empty() {
                return Ok("(no results)\n".to_string());
            }

            let mut widths: Vec<usize> = column_names.iter().map(|n| n.len()).collect();
            for row in &collected {
                for (i, name) in column_names.iter().enumerate() {
                    let cell = match row.get(name) {
                        Some(Value::String(s)) => s.len(),
                        Some(Value::Null) | None => 0,
                        Some(v) => v.to_string().len(),
                    };
                    if cell > widths[i] {
                        widths[i] = cell;
                    }
                }
            }

            let mut out = String::new();
            let header: Vec<String> = column_names
                .iter()
                .enumerate()
                .map(|(i, n)| format!("{:<w$}", n, w = widths[i]))
                .collect();
            out.push_str(&header.join("  "));
            out.push('\n');

            let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            out.push_str(&sep.join("  "));
            out.push('\n');

            for row in &collected {
                let cells: Vec<String> = column_names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let cell = match row.get(name) {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Null) | None => String::new(),
                            Some(v) => v.to_string(),
                        };
                        format!("{:<w$}", cell, w = widths[i])
                    })
                    .collect();
                out.push_str(&cells.join("  "));
                out.push('\n');
            }

            Ok(out)
        }
    }
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

    if let Err(e) = run_parse(&canonical, &data_dir, lang_filter) {
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
