use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use anyhow::Result;
use rayon::prelude::*;

use crate::language::Language;
use crate::languages;
use crate::models::SymbolInfo;
use crate::parser;
use crate::query_engine::QueryResult;
use crate::signature;
use crate::workspace::Workspace;

/// A directed edge in the call graph
#[derive(Debug, Clone)]
pub struct CallEdge {
    pub caller_name: String,
    pub caller_file: String,
    pub caller_line: u32,
    pub callee_name: String,
}

/// Find call expression names within a symbol's line range.
pub fn find_callees_in_source(source: &str, sym: &SymbolInfo, language: Language) -> Vec<String> {
    let call_node_types = call_expression_types(language);
    let mut ts_parser = match parser::create_parser(language) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let tree = match ts_parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut callees = Vec::new();
    let source_bytes = source.as_bytes();

    collect_calls_in_range(
        tree.root_node(),
        source_bytes,
        sym.start_line,
        sym.end_line,
        &call_node_types,
        language,
        &mut callees,
    );

    // Deduplicate
    let mut seen = HashSet::new();
    callees.retain(|c| seen.insert(c.clone()));
    callees
}

fn collect_calls_in_range(
    node: tree_sitter::Node,
    source: &[u8],
    start_line: u32,
    end_line: u32,
    call_types: &[&str],
    language: Language,
    out: &mut Vec<String>,
) {
    let node_start = node.start_position().row as u32 + 1;
    let node_end = node.end_position().row as u32 + 1;

    // Skip nodes entirely outside our range
    if node_end < start_line || node_start > end_line {
        return;
    }

    if call_types.contains(&node.kind())
        && let Some(name) = extract_callee_name(node, source, language)
    {
        out.push(name);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_in_range(
            child, source, start_line, end_line, call_types, language, out,
        );
    }
}

fn extract_callee_name(
    node: tree_sitter::Node,
    source: &[u8],
    _language: Language,
) -> Option<String> {
    // The function/callee is typically the first child
    let func_node = node
        .child_by_field_name("function")
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child(0))?;

    let text = func_node.utf8_text(source).ok()?;

    // For method calls like obj.method(), extract just "method"
    let name = if let Some(pos) = text.rfind('.') {
        &text[pos + 1..]
    } else if let Some(pos) = text.rfind("::") {
        &text[pos + 2..]
    } else {
        text
    };

    // Clean up: remove parens, trim
    let name = name.trim_end_matches('(').trim();

    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn call_expression_types(language: Language) -> Vec<&'static str> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            vec!["call_expression", "new_expression"]
        }
        Language::Rust => vec!["call_expression", "method_call_expression"],
        Language::Python => vec!["call"],
        Language::Go => vec!["call_expression"],
        Language::Java => vec!["method_invocation", "object_creation_expression"],
        Language::C | Language::Cpp => vec!["call_expression"],
        Language::CSharp => vec!["invocation_expression", "object_creation_expression"],
        Language::Php => vec!["function_call_expression", "method_call_expression"],
    }
}

/// Traverse the call graph starting from seed symbols.
/// Returns results at each depth level up to `max_depth`.
pub fn traverse_call_graph(
    workspace: &Workspace,
    seeds: &[QueryResult],
    direction: &str,
    max_depth: usize,
) -> Result<Vec<QueryResult>> {
    // Pre-compile queries for loaded languages
    let mut sym_queries_map = HashMap::new();
    for rel_path in workspace.files() {
        if let Some(lang) = workspace.file_language(rel_path)
            && let std::collections::hash_map::Entry::Vacant(e) = sym_queries_map.entry(lang)
        {
            e.insert(languages::compile_symbol_query(lang)?);
        }
    }
    let sym_queries = Arc::new(sym_queries_map);

    // Parse all files in parallel, building symbol and source maps
    let file_data: Vec<_> = workspace
        .files()
        .par_iter()
        .filter_map(|rel_path| {
            let lang = workspace.file_language(rel_path)?;
            let sym_query = sym_queries.get(&lang)?;
            let source = workspace.read_file(rel_path)?;
            let mut ts_parser = parser::create_parser(lang).ok()?;
            let (metadata, tree) =
                parser::parse_content(&mut ts_parser, &source, rel_path, lang).ok()?;
            let symbols = languages::extract_symbols(
                &tree,
                source.as_bytes(),
                sym_query,
                &metadata.path,
                lang,
            );
            Some((metadata.path.clone(), source, symbols, lang))
        })
        .collect();

    // Build indexes
    let mut source_by_file: HashMap<&str, &str> = HashMap::new();
    let mut symbols_by_file: HashMap<&str, &[SymbolInfo]> = HashMap::new();
    let mut lang_by_file: HashMap<&str, Language> = HashMap::new();
    let mut symbols_by_name: HashMap<&str, Vec<(&str, &SymbolInfo)>> = HashMap::new();

    for (file, source, symbols, lang) in &file_data {
        let source_ref: &str = source;
        source_by_file.insert(file.as_str(), source_ref);
        symbols_by_file.insert(file.as_str(), symbols.as_slice());
        lang_by_file.insert(file.as_str(), *lang);
        for sym in symbols {
            symbols_by_name
                .entry(sym.name.as_str())
                .or_default()
                .push((file.as_str(), sym));
        }
    }

    let mut visited: HashSet<(String, u32)> = HashSet::new();
    let mut results: Vec<QueryResult> = Vec::new();
    let mut queue: VecDeque<(String, String, u32, usize)> = VecDeque::new(); // (name, file, line, depth)

    // Seed the queue
    for seed in seeds {
        queue.push_back((seed.name.clone(), seed.file.clone(), seed.line, 0));
        visited.insert((seed.name.clone(), seed.line));
    }

    while let Some((name, file, line, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        match direction {
            "down" => {
                // Find callees of this symbol
                if let (Some(source), Some(lang)) = (
                    source_by_file.get(file.as_str()),
                    lang_by_file.get(file.as_str()),
                ) {
                    // Find the symbol in the file
                    if let Some(symbols) = symbols_by_file.get(file.as_str())
                        && let Some(sym) = symbols
                            .iter()
                            .find(|s| s.name == name && s.start_line == line)
                    {
                        let callees = find_callees_in_source(source, sym, *lang);
                        for callee_name in callees {
                            // Resolve callee to actual symbols
                            if let Some(targets) = symbols_by_name.get(callee_name.as_str()) {
                                for (target_file, target_sym) in targets {
                                    let key = (target_sym.name.clone(), target_sym.start_line);
                                    if visited.insert(key) {
                                        let sig = source_by_file.get(target_file).and_then(|s| {
                                            signature::extract_signature(
                                                s,
                                                target_sym.start_line,
                                                *lang_by_file
                                                    .get(target_file)
                                                    .unwrap_or(&Language::TypeScript),
                                            )
                                        });

                                        results.push(QueryResult {
                                            name: target_sym.name.clone(),
                                            kind: target_sym.kind.to_string(),
                                            file: target_file.to_string(),
                                            line: target_sym.start_line,
                                            end_line: target_sym.end_line,
                                            column: target_sym.start_column,
                                            exported: target_sym.is_exported,
                                            signature: sig,
                                            docstring: None,
                                            body: None,
                                            preview: None,
                                            parent: None,
                                        });

                                        queue.push_back((
                                            target_sym.name.clone(),
                                            target_file.to_string(),
                                            target_sym.start_line,
                                            depth + 1,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "up" => {
                // Find callers: scan all files for call expressions that reference `name`
                for (file_path, source, symbols, lang) in &file_data {
                    let source_ref: &str = source;
                    for sym in symbols.iter() {
                        let key = (sym.name.clone(), sym.start_line);
                        if visited.contains(&key) {
                            continue;
                        }

                        let callees = find_callees_in_source(source_ref, sym, *lang);
                        if callees.iter().any(|c| c == &name) && visited.insert(key) {
                            let sig =
                                signature::extract_signature(source_ref, sym.start_line, *lang);

                            results.push(QueryResult {
                                name: sym.name.clone(),
                                kind: sym.kind.to_string(),
                                file: file_path.clone(),
                                line: sym.start_line,
                                end_line: sym.end_line,
                                column: sym.start_column,
                                exported: sym.is_exported,
                                signature: sig,
                                docstring: None,
                                body: None,
                                preview: None,
                                parent: None,
                            });

                            queue.push_back((
                                sym.name.clone(),
                                file_path.clone(),
                                sym.start_line,
                                depth + 1,
                            ));
                        }
                    }
                }
            }
            "both" => {
                // Do both directions: first down, then up
                // Down
                if let (Some(source), Some(lang)) = (
                    source_by_file.get(file.as_str()),
                    lang_by_file.get(file.as_str()),
                ) && let Some(symbols) = symbols_by_file.get(file.as_str())
                    && let Some(sym) = symbols
                        .iter()
                        .find(|s| s.name == name && s.start_line == line)
                {
                    let callees = find_callees_in_source(source, sym, *lang);
                    for callee_name in callees {
                        if let Some(targets) = symbols_by_name.get(callee_name.as_str()) {
                            for (target_file, target_sym) in targets {
                                let key = (target_sym.name.clone(), target_sym.start_line);
                                if visited.insert(key) {
                                    let sig = source_by_file.get(target_file).and_then(|s| {
                                        signature::extract_signature(
                                            s,
                                            target_sym.start_line,
                                            *lang_by_file
                                                .get(target_file)
                                                .unwrap_or(&Language::TypeScript),
                                        )
                                    });

                                    results.push(QueryResult {
                                        name: target_sym.name.clone(),
                                        kind: target_sym.kind.to_string(),
                                        file: target_file.to_string(),
                                        line: target_sym.start_line,
                                        end_line: target_sym.end_line,
                                        column: target_sym.start_column,
                                        exported: target_sym.is_exported,
                                        signature: sig,
                                        docstring: None,
                                        body: None,
                                        preview: None,
                                        parent: None,
                                    });

                                    queue.push_back((
                                        target_sym.name.clone(),
                                        target_file.to_string(),
                                        target_sym.start_line,
                                        depth + 1,
                                    ));
                                }
                            }
                        }
                    }
                }

                // Up
                for (file_path, source, symbols, lang) in &file_data {
                    let source_ref: &str = source;
                    for sym in symbols.iter() {
                        let key = (sym.name.clone(), sym.start_line);
                        if visited.contains(&key) {
                            continue;
                        }

                        let callees = find_callees_in_source(source_ref, sym, *lang);
                        if callees.iter().any(|c| c == &name) && visited.insert(key) {
                            let sig =
                                signature::extract_signature(source_ref, sym.start_line, *lang);

                            results.push(QueryResult {
                                name: sym.name.clone(),
                                kind: sym.kind.to_string(),
                                file: file_path.clone(),
                                line: sym.start_line,
                                end_line: sym.end_line,
                                column: sym.start_column,
                                exported: sym.is_exported,
                                signature: sig,
                                docstring: None,
                                body: None,
                                preview: None,
                                parent: None,
                            });

                            queue.push_back((
                                sym.name.clone(),
                                file_path.clone(),
                                sym.start_line,
                                depth + 1,
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    results.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    Ok(results)
}
