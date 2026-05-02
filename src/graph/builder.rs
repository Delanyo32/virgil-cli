use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use rayon::prelude::*;
use tree_sitter::Query;

use crate::language::Language;
use crate::languages;
use crate::models::{ImportInfo, SymbolInfo, SymbolKind};
use crate::parser;
use crate::storage::workspace::Workspace;

use super::cfg::FunctionCfg;
use super::cfg_languages;
use super::{CodeGraph, EdgeWeight, NodeWeight};

/// Per-file extraction result, collected in parallel.
struct FileGraphData {
    path: String,
    language: Language,
    symbols: Vec<SymbolInfo>,
    imports: Vec<ImportInfo>,
    call_sites: Vec<CallSiteData>,
    /// CFGs built for functions in this file: (symbol start_line, cfg)
    function_cfgs: Vec<(u32, FunctionCfg)>,
}

/// A call site extracted from within a symbol's line range.
struct CallSiteData {
    callee_name: String,
    caller_file: String,
    caller_symbol_line: u32,
}

pub struct GraphBuilder<'a> {
    workspace: &'a Workspace,
    languages: &'a [Language],
}

impl<'a> GraphBuilder<'a> {
    pub fn new(workspace: &'a Workspace, languages: &'a [Language]) -> Self {
        Self {
            workspace,
            languages,
        }
    }

    pub fn build(&self) -> Result<CodeGraph> {
        // Step 1: Pre-compile queries per language
        let mut symbol_queries: HashMap<Language, Arc<Query>> = HashMap::new();
        let mut import_queries: HashMap<Language, Arc<Query>> = HashMap::new();
        for &lang in self.languages {
            symbol_queries.insert(lang, languages::compile_symbol_query(lang)?);
            import_queries.insert(lang, languages::compile_import_query(lang)?);
        }
        let symbol_queries = Arc::new(symbol_queries);
        let import_queries = Arc::new(import_queries);

        // Step 2: Build known_files set
        let known_files: HashSet<String> = self.workspace.files().iter().cloned().collect();

        // Step 3: Group files by language
        let grouped_files: Vec<(Language, &str)> = self
            .workspace
            .files()
            .iter()
            .filter_map(|path| {
                let lang = self.workspace.file_language(path)?;
                if symbol_queries.contains_key(&lang) {
                    Some((lang, path.as_str()))
                } else {
                    None
                }
            })
            .collect();

        // Step 4: Parallel parse + extract per file
        let pool = rayon::ThreadPoolBuilder::new()
            .stack_size(4 * 1024 * 1024)
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

        let file_data: Vec<FileGraphData> = pool.install(|| {
            grouped_files
                .par_iter()
                .filter_map(|&(lang, rel_path)| {
                    let sym_query = symbol_queries.get(&lang)?;
                    let imp_query = import_queries.get(&lang)?;

                    let mut ts_parser = parser::create_parser(lang).ok()?;
                    let source = self.workspace.read_file(rel_path)?;
                    let tree = ts_parser.parse(&*source, None)?;

                    let symbols = languages::extract_symbols(
                        &tree,
                        source.as_bytes(),
                        sym_query,
                        rel_path,
                        lang,
                    );
                    let imports = languages::extract_imports(
                        &tree,
                        source.as_bytes(),
                        imp_query,
                        rel_path,
                        lang,
                    );

                    // Extract call sites within each symbol's line range
                    let call_node_types = call_expression_types(lang);
                    let mut call_sites = Vec::new();
                    for sym in &symbols {
                        collect_calls_in_range(
                            tree.root_node(),
                            source.as_bytes(),
                            sym.start_line,
                            sym.end_line,
                            &call_node_types,
                            lang,
                            rel_path,
                            sym.start_line,
                            &mut call_sites,
                        );
                    }

                    // Build CFGs for functions in this file
                    let function_cfgs =
                        build_function_cfgs(&tree, source.as_bytes(), &symbols, lang);

                    Some(FileGraphData {
                        path: rel_path.to_string(),
                        language: lang,
                        symbols,
                        imports,
                        call_sites,
                        function_cfgs,
                    })
                })
                .collect()
        });

        // Step 5: Assemble graph (single-threaded — DiGraph is not Sync)
        let mut graph = CodeGraph::new();

        // 5a: Add File nodes
        for fd in &file_data {
            let file_idx = graph.graph.add_node(NodeWeight::File {
                path: fd.path.clone(),
                language: fd.language,
            });
            graph.file_nodes.insert(fd.path.clone(), file_idx);
        }

        // 5b: Add Symbol nodes + DefinedIn/Exports/Contains edges
        for fd in &file_data {
            let file_idx = graph.file_nodes[&fd.path];
            for sym in &fd.symbols {
                let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
                    name: sym.name.clone(),
                    kind: sym.kind,
                    file_path: sym.file_path.clone(),
                    start_line: sym.start_line,
                    end_line: sym.end_line,
                    exported: sym.is_exported,
                });

                // DefinedIn: Symbol -> File
                graph
                    .graph
                    .add_edge(sym_idx, file_idx, EdgeWeight::DefinedIn);

                // Contains: File -> Symbol
                graph
                    .graph
                    .add_edge(file_idx, sym_idx, EdgeWeight::Contains);

                // Exports: File -> Symbol (if exported)
                if sym.is_exported {
                    graph.graph.add_edge(file_idx, sym_idx, EdgeWeight::Exports);
                }

                graph
                    .symbol_nodes
                    .insert((sym.file_path.clone(), sym.start_line), sym_idx);
                graph
                    .symbols_by_name
                    .entry(sym.name.clone())
                    .or_default()
                    .push(sym_idx);
            }
        }

        // 5c: Resolve imports -> Imports edges between file nodes
        for fd in &file_data {
            let from_file_idx = graph.file_nodes[&fd.path];
            for import in &fd.imports {
                if let Some(resolved) =
                    resolve_import_to_file(&fd.path, import, fd.language, &known_files)
                    && let Some(&to_file_idx) = graph.file_nodes.get(&resolved)
                    && from_file_idx != to_file_idx
                {
                    graph
                        .graph
                        .add_edge(from_file_idx, to_file_idx, EdgeWeight::Imports);
                }
            }
        }

        // 5d: Resolve calls via symbols_by_name -> Calls edges
        for fd in &file_data {
            for cs in &fd.call_sites {
                // Find the caller symbol node
                let caller_key = (cs.caller_file.clone(), cs.caller_symbol_line);
                let caller_idx = match graph.symbol_nodes.get(&caller_key) {
                    Some(&idx) => idx,
                    None => continue,
                };

                // Find target symbols by name
                if let Some(targets) = graph.symbols_by_name.get(&cs.callee_name) {
                    for &target_idx in targets {
                        if target_idx != caller_idx {
                            graph
                                .graph
                                .add_edge(caller_idx, target_idx, EdgeWeight::Calls);
                        }
                    }
                }
            }
        }

        // 5e: Store function CFGs
        for fd in &file_data {
            for (start_line, cfg) in &fd.function_cfgs {
                if let Some(&sym_idx) = graph.symbol_nodes.get(&(fd.path.clone(), *start_line)) {
                    graph.function_cfgs.insert(sym_idx, cfg.clone());
                }
            }
        }

        // Step 6: Resource lifecycle analysis — compute Acquires/ReleasedBy edges
        // Note: taint analysis is no longer run at build time. It is invoked by the
        // executor when processing a GraphStage::Taint pipeline stage, using patterns
        // supplied by the JSON pipeline file (TaintConfig).
        super::resource::ResourceAnalyzer::analyze_all(&mut graph);

        Ok(graph)
    }
}

/// Resolve an import to a file path string (unwrapping GraphNode).
fn resolve_import_to_file(
    source_file: &str,
    import: &ImportInfo,
    language: Language,
    known_files: &HashSet<String>,
) -> Option<String> {
    use crate::graph::project_index::GraphNode;
    let node = languages::resolve_import(source_file, import, language, known_files)?;
    Some(match node {
        GraphNode::File(p) => p,
        GraphNode::Package(p) => p,
    })
}

// --- CFG building ---

/// Build CFGs for all function/method symbols in a parsed file.
fn build_function_cfgs(
    tree: &tree_sitter::Tree,
    source: &[u8],
    symbols: &[SymbolInfo],
    language: Language,
) -> Vec<(u32, FunctionCfg)> {
    let builder = match cfg_languages::cfg_builder_for_language(language) {
        Some(b) => b,
        None => return Vec::new(),
    };

    let mut cfgs = Vec::new();

    for sym in symbols {
        // Only build CFGs for function-like symbols
        match sym.kind {
            SymbolKind::Function | SymbolKind::Method | SymbolKind::ArrowFunction => {}
            _ => continue,
        }

        // Find the tree-sitter node for this function by line range
        if let Some(func_node) = find_node_at_line(tree.root_node(), sym.start_line, sym.end_line)
            && let Ok(cfg) = builder.build_cfg(&func_node, source)
        {
            cfgs.push((sym.start_line, cfg));
        }
    }

    cfgs
}

/// Find a tree-sitter node that matches the given line range.
fn find_node_at_line(
    node: tree_sitter::Node,
    start_line: u32,
    end_line: u32,
) -> Option<tree_sitter::Node> {
    let node_start = node.start_position().row as u32 + 1;
    let node_end = node.end_position().row as u32 + 1;

    if node_start == start_line && node_end == end_line {
        return Some(node);
    }

    // Skip nodes that don't contain the target
    if node_end < start_line || node_start > end_line {
        return None;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_node_at_line(child, start_line, end_line) {
            return Some(found);
        }
    }

    None
}

// --- Call extraction (ported from call_graph.rs) ---

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

#[allow(clippy::too_many_arguments)]
fn collect_calls_in_range(
    node: tree_sitter::Node,
    source: &[u8],
    start_line: u32,
    end_line: u32,
    call_types: &[&str],
    language: Language,
    file_path: &str,
    caller_symbol_line: u32,
    out: &mut Vec<CallSiteData>,
) {
    let node_start = node.start_position().row as u32 + 1;
    let node_end = node.end_position().row as u32 + 1;

    if node_end < start_line || node_start > end_line {
        return;
    }

    if call_types.contains(&node.kind())
        && let Some(name) = extract_callee_name(node, source, language)
    {
        out.push(CallSiteData {
            callee_name: name,
            caller_file: file_path.to_string(),
            caller_symbol_line,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_in_range(
            child,
            source,
            start_line,
            end_line,
            call_types,
            language,
            file_path,
            caller_symbol_line,
            out,
        );
    }
}

fn extract_callee_name(
    node: tree_sitter::Node,
    source: &[u8],
    _language: Language,
) -> Option<String> {
    let func_node = node
        .child_by_field_name("function")
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child(0))?;

    let text = func_node.utf8_text(source).ok()?;

    let name = if let Some(pos) = text.rfind('.') {
        &text[pos + 1..]
    } else if let Some(pos) = text.rfind("::") {
        &text[pos + 2..]
    } else {
        text
    };

    let name = name.trim_end_matches('(').trim();

    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}
