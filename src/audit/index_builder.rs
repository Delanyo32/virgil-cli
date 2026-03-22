use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use rayon::prelude::*;
use tree_sitter::Query;

use crate::language::Language;
use crate::languages;
use crate::parser;
use crate::signature;
use crate::workspace::Workspace;

use super::project_index::{ExportedSymbol, FileEntry, GraphNode, ProjectIndex};

/// Build a cross-file index from a workspace.
/// Performs a full tree-sitter parse pass, extracts imports + symbols, resolves imports.
pub fn build_index(workspace: &Workspace, languages_filter: &[Language]) -> Result<ProjectIndex> {
    // Step 1: Pre-compile queries per language
    let mut symbol_queries: HashMap<Language, Arc<Query>> = HashMap::new();
    let mut import_queries: HashMap<Language, Arc<Query>> = HashMap::new();
    for &lang in languages_filter {
        symbol_queries.insert(lang, languages::compile_symbol_query(lang)?);
        import_queries.insert(lang, languages::compile_import_query(lang)?);
    }
    let symbol_queries = Arc::new(symbol_queries);
    let import_queries = Arc::new(import_queries);

    // Step 2: Build known_files set for resolvers
    let known_files: HashSet<String> = workspace.files().iter().cloned().collect();
    let known_files_arc = Arc::new(known_files);

    // Step 3: Group files by language
    let grouped_files: Vec<(Language, &str)> = workspace
        .files()
        .iter()
        .filter_map(|path| {
            let lang = workspace.file_language(path)?;
            if symbol_queries.contains_key(&lang) {
                Some((lang, path.as_str()))
            } else {
                None
            }
        })
        .collect();

    // Step 4: Parallel parse + extract
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(4 * 1024 * 1024)
        .build()
        .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

    let file_entries: Vec<FileEntry> = pool.install(|| {
        grouped_files
            .par_iter()
            .filter_map(|&(lang, rel_path)| {
                let sym_query = symbol_queries.get(&lang)?;
                let imp_query = import_queries.get(&lang)?;

                let mut ts_parser = parser::create_parser(lang).ok()?;
                let source = workspace.read_file(rel_path)?;
                let tree = ts_parser.parse(&*source, None)?;

                let symbols =
                    languages::extract_symbols(&tree, source.as_bytes(), sym_query, rel_path, lang);
                let imports =
                    languages::extract_imports(&tree, source.as_bytes(), imp_query, rel_path, lang);

                let line_count = source.lines().count() as u32;
                let exported_symbols: Vec<ExportedSymbol> = symbols
                    .iter()
                    .filter(|s| s.is_exported)
                    .map(|s| ExportedSymbol {
                        name: s.name.clone(),
                        kind: s.kind,
                        signature: signature::extract_signature(&source, s.start_line, lang),
                        start_line: s.start_line,
                    })
                    .collect();

                Some(FileEntry {
                    path: rel_path.to_string(),
                    language: lang,
                    line_count,
                    symbol_count: symbols.len(),
                    exported_symbols,
                    imports,
                })
            })
            .collect()
    });

    // Step 5: Build graph edges using resolvers
    let mut index = ProjectIndex::new();
    index.known_files = Arc::try_unwrap(known_files_arc).unwrap_or_else(|arc| (*arc).clone());

    for entry in &file_entries {
        let from_node = if entry.language == Language::Go {
            let dir = entry.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            GraphNode::Package(dir.to_string())
        } else {
            GraphNode::File(entry.path.clone())
        };

        for import in &entry.imports {
            if let Some(to_node) = languages::resolve_import(
                &entry.path,
                import,
                entry.language,
                &index.known_files,
            ) && from_node != to_node {
                index
                    .edges
                    .entry(from_node.clone())
                    .or_default()
                    .insert(to_node);
            }
        }
    }

    // Store file entries
    for entry in file_entries {
        index.files.insert(entry.path.clone(), entry);
    }

    Ok(index)
}
