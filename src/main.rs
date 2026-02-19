use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;

use virgil_cli::cli::{Cli, Command, OutputFormat};
use virgil_cli::discovery;
use virgil_cli::language::{self, Language};
use virgil_cli::languages;
use virgil_cli::models::{CommentInfo, FileMetadata, ImportInfo, SymbolInfo};
use virgil_cli::output;
use virgil_cli::parser;
use virgil_cli::query;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Parse {
            dir,
            output: output_dir,
            language: lang_filter,
        } => run_parse(&dir, &output_dir, lang_filter.as_deref()),

        Command::Overview { data_dir, format, depth } => {
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
            let output = query::read::run_read(&file_path, &root, start_line, end_line)?;
            print!("{output}");
            Ok(())
        }

        Command::Query {
            sql,
            data_dir,
            format,
        } => {
            let engine = query::db::QueryEngine::new(&data_dir)?;
            let output = run_raw_query(&engine, &sql, &format)?;
            print!("{output}");
            Ok(())
        }

        Command::Deps {
            file_path,
            data_dir,
            format,
        } => {
            let engine = query::db::QueryEngine::new(&data_dir)?;
            let output = query::deps::run_deps(&engine, &file_path, &format)?;
            print!("{output}");
            Ok(())
        }

        Command::Dependents {
            file_path,
            data_dir,
            format,
        } => {
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
            let engine = query::db::QueryEngine::new(&data_dir)?;
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
    }
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

    // Phase 1: Discover files
    let files = discovery::discover_files(&root, &languages)?;
    eprintln!("Discovered {} files", files.len());

    if files.is_empty() {
        eprintln!("No files found. Nothing to do.");
        return Ok(());
    }

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

    // Phase 2-3: Parse files and extract symbols + imports + comments (parallel)
    let results: Vec<_> = files
        .par_iter()
        .filter_map(|path| {
            let ext = path.extension()?.to_str()?;
            let lang = Language::from_extension(ext)?;
            let query = query_map.get(&lang)?;
            let import_query = import_query_map.get(&lang)?;
            let comment_query = comment_query_map.get(&lang)?;

            let mut ts_parser = match parser::create_parser(lang) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Warning: failed to create parser for {}: {e}", path.display());
                    return None;
                }
            };

            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Warning: failed to read {}: {e}", path.display());
                    return None;
                }
            };

            let (metadata, tree) = match parser::parse_file(&mut ts_parser, path, &root, lang) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {e}", path.display());
                    return None;
                }
            };

            let syms = languages::extract_symbols(&tree, source.as_bytes(), query, &metadata.path, lang);
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

            Some((metadata, syms, imps, cmts))
        })
        .collect();

    let mut all_files: Vec<FileMetadata> = Vec::new();
    let mut all_symbols: Vec<SymbolInfo> = Vec::new();
    let mut all_imports: Vec<ImportInfo> = Vec::new();
    let mut all_comments: Vec<CommentInfo> = Vec::new();

    for (metadata, syms, imps, cmts) in results {
        all_files.push(metadata);
        all_symbols.extend(syms);
        all_imports.extend(imps);
        all_comments.extend(cmts);
    }

    // Phase 4: Write parquet output
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output dir: {}", output_dir.display()))?;

    output::write_files_parquet(&all_files, output_dir)?;
    output::write_symbols_parquet(&all_symbols, output_dir)?;
    output::write_imports_parquet(&all_imports, output_dir)?;
    output::write_comments_parquet(&all_comments, output_dir)?;

    let elapsed = start.elapsed();
    eprintln!(
        "Done: {} files, {} symbols, {} imports, {} comments in {:.2}s",
        all_files.len(),
        all_symbols.len(),
        all_imports.len(),
        all_comments.len(),
        elapsed.as_secs_f64()
    );
    eprintln!(
        "Output: {}/{{files,symbols,imports,comments}}.parquet",
        output_dir.display(),
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
        for i in 0..column_count {
            let name = &column_names[i];
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
                Ok(ValueRef::Blob(bytes)) => {
                    Value::String(format!("<blob {} bytes>", bytes.len()))
                }
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
