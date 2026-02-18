use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;

use virgil_cli::cli::Args;
use virgil_cli::discovery;
use virgil_cli::language::{self, Language};
use virgil_cli::models::{FileMetadata, SymbolInfo};
use virgil_cli::output;
use virgil_cli::parser;
use virgil_cli::symbols;

fn main() -> Result<()> {
    let args = Args::parse();

    let root = args
        .dir
        .canonicalize()
        .with_context(|| format!("invalid directory: {}", args.dir.display()))?;

    let languages: Vec<Language> = if let Some(ref filter) = args.language {
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
    for lang in &languages {
        query_map.insert(*lang, symbols::compile_query(*lang)?);
    }
    let query_map = Arc::new(query_map);

    // Phase 2-3: Parse files and extract symbols (parallel)
    let results: Vec<_> = files
        .par_iter()
        .filter_map(|path| {
            let ext = path.extension()?.to_str()?;
            let lang = Language::from_extension(ext)?;
            let query = query_map.get(&lang)?;

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

            let syms = symbols::extract_symbols(&tree, source.as_bytes(), query, &metadata.path);

            Some((metadata, syms))
        })
        .collect();

    let mut all_files: Vec<FileMetadata> = Vec::new();
    let mut all_symbols: Vec<SymbolInfo> = Vec::new();

    for (metadata, syms) in results {
        all_files.push(metadata);
        all_symbols.extend(syms);
    }

    // Phase 4: Write parquet output
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("failed to create output dir: {}", args.output.display()))?;

    output::write_files_parquet(&all_files, &args.output)?;
    output::write_symbols_parquet(&all_symbols, &args.output)?;

    let elapsed = start.elapsed();
    eprintln!(
        "Done: {} files, {} symbols in {:.2}s",
        all_files.len(),
        all_symbols.len(),
        elapsed.as_secs_f64()
    );
    eprintln!(
        "Output: {}/files.parquet, {}/symbols.parquet",
        args.output.display(),
        args.output.display()
    );

    Ok(())
}
