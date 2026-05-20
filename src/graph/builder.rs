use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use anyhow::Result;
use petgraph::graph::NodeIndex;
use rayon::prelude::*;
use tree_sitter::Query;

use crate::language::Language;
use crate::languages;
use crate::models::{ImportInfo, SymbolInfo, SymbolKind};
use crate::parser;
use crate::storage::workspace::Workspace;

use super::cfg::{CfgStatementKind, FunctionCfg};
use super::{CfgExitKind, CodeGraph, EdgeWeight, NodeWeight, Spur};

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
    /// Line of the call expression itself (1-based).
    line: u32,
    /// Literal arguments (strings/numbers/bools) — non-literal args are skipped.
    arg_literals: Vec<String>,
    /// Name of the enclosing test function, when the caller is inside one.
    enclosing_test_name: Option<String>,
}

/// An import deferred until all File nodes are present.
struct DeferredImport {
    from_file_path: String,
    language: Language,
    import: ImportInfo,
}

/// A Calls-edge deferred until all Symbol nodes are present.
struct DeferredCall {
    caller_idx: NodeIndex,
    callee_name: String,
}

/// Knobs for skipping passes the caller does not need. Defaults run every
/// pass (current behaviour). Skipping a pass keeps the corresponding edges
/// out of the graph entirely — any audit pipeline that depends on them will
/// silently produce no findings.
#[derive(Debug, Clone, Copy)]
pub struct BuildOptions {
    /// When false, per-function CFGs are not built. This disables `ExitsVia`
    /// edges, taint analysis (no functions to walk), and the lifecycle pass.
    pub build_cfgs: bool,
    /// When false, the lifecycle pass that emits `Acquires` / `ReleasedBy`
    /// edges is suppressed even on explicit `ensure_resource_graph` calls.
    pub build_resource_graph: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            build_cfgs: true,
            build_resource_graph: true,
        }
    }
}

pub struct GraphBuilder<'a> {
    workspace: &'a Workspace,
    languages: &'a [Language],
    options: BuildOptions,
}

impl<'a> GraphBuilder<'a> {
    pub fn new(workspace: &'a Workspace, languages: &'a [Language]) -> Self {
        Self {
            workspace,
            languages,
            options: BuildOptions::default(),
        }
    }

    pub fn with_options(mut self, options: BuildOptions) -> Self {
        self.options = options;
        self
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

        // Step 4 + 5: Parallel parse, single-threaded streaming absorption.
        //
        // Parse workers send each `FileGraphData` to the drainer over a
        // bounded channel — backpressure caps peak memory at roughly
        // `2 * num_cpus` in-flight `FileGraphData` values, instead of letting
        // a slow drainer accumulate the whole workspace in the queue. The
        // drainer absorbs file-local edges (File/Symbol/CallSite nodes,
        // ExitsVia) immediately and drops the message, so the intermediate
        // `FunctionCfg`s and call-site metadata don't pile up.
        //
        // Cross-file references (imports, Calls edges) need every symbol to
        // be registered first, so they are buffered as small `Deferred*`
        // tuples and resolved after the channel drains.
        let pool = rayon::ThreadPoolBuilder::new()
            .stack_size(4 * 1024 * 1024)
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

        let mut graph = CodeGraph::new();
        let mut deferred_imports: Vec<DeferredImport> = Vec::new();
        let mut deferred_calls: Vec<DeferredCall> = Vec::new();

        let workspace = self.workspace;
        let sym_q = Arc::clone(&symbol_queries);
        let imp_q = Arc::clone(&import_queries);
        let grouped_files_ref = &grouped_files;
        let build_cfgs = self.options.build_cfgs;

        thread::scope(|s| -> Result<()> {
            let parallelism = thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            let channel_bound = (parallelism * 2).max(4);
            let (tx, rx) = mpsc::sync_channel::<FileGraphData>(channel_bound);

            s.spawn(move || {
                pool.install(|| {
                    grouped_files_ref
                        .par_iter()
                        .for_each_with(tx, |tx, &(lang, rel_path)| {
                            if let Some(data) = parse_one_file(
                                lang, rel_path, workspace, &sym_q, &imp_q, build_cfgs,
                            ) {
                                let _ = tx.send(data);
                            }
                        });
                });
            });

            while let Ok(data) = rx.recv() {
                absorb_file_data(&mut graph, data, &mut deferred_imports, &mut deferred_calls);
            }

            Ok(())
        })?;

        // Resolve deferred imports now that every File node exists.
        for di in deferred_imports {
            let Some(from_spur) = graph.symbols.get(&di.from_file_path) else {
                continue;
            };
            let Some(&from_file_idx) = graph.file_nodes.get(&from_spur) else {
                continue;
            };
            if let Some(resolved) =
                resolve_import_to_file(&di.from_file_path, &di.import, di.language, &known_files)
                && let Some(to_spur) = graph.symbols.get(&resolved)
                && let Some(&to_file_idx) = graph.file_nodes.get(&to_spur)
                && from_file_idx != to_file_idx
            {
                graph
                    .graph
                    .add_edge(from_file_idx, to_file_idx, EdgeWeight::Imports);
            }
        }

        // Resolve deferred Calls edges now that every Symbol is registered.
        for dc in deferred_calls {
            let Some(callee_spur) = graph.symbols.get(&dc.callee_name) else {
                continue;
            };
            if let Some(targets) = graph.symbols_by_name.get(&callee_spur) {
                for &target_idx in targets {
                    if target_idx != dc.caller_idx {
                        graph
                            .graph
                            .add_edge(dc.caller_idx, target_idx, EdgeWeight::Calls);
                    }
                }
            }
        }

        // Resource lifecycle analysis is no longer run here. Callers that need
        // `Acquires` / `ReleasedBy` edges call `graph.ensure_resource_graph(workspace)`
        // explicitly (CLI flows) or trigger the lazy populate path on `AppState`
        // (serve flow). This keeps boot-time memory and CPU off the critical path
        // for callers that never query lifecycle edges.

        Ok(graph)
    }
}

/// Parse a single file and produce its `FileGraphData`. Runs on a rayon
/// worker; the parser instance is local and dropped on return.
fn parse_one_file(
    lang: Language,
    rel_path: &str,
    workspace: &Workspace,
    symbol_queries: &HashMap<Language, Arc<Query>>,
    import_queries: &HashMap<Language, Arc<Query>>,
    build_cfgs: bool,
) -> Option<FileGraphData> {
    let sym_query = symbol_queries.get(&lang)?;
    let imp_query = import_queries.get(&lang)?;

    let mut ts_parser = parser::create_parser(lang).ok()?;
    let source = workspace.read_file(rel_path)?;
    let tree = ts_parser.parse(&*source, None)?;

    let symbols = languages::extract_symbols(&tree, source.as_bytes(), sym_query, rel_path, lang);
    let imports = languages::extract_imports(&tree, source.as_bytes(), imp_query, rel_path, lang);

    let call_node_types = call_expression_types(lang);
    let is_test = crate::classify::is_test_file(rel_path);
    let mut call_sites = Vec::new();
    for sym in &symbols {
        let enclosing_test = if is_test && is_test_function_name(&sym.name) {
            Some(sym.name.clone())
        } else {
            None
        };
        collect_calls_in_range(
            tree.root_node(),
            source.as_bytes(),
            sym.start_line,
            sym.end_line,
            &call_node_types,
            lang,
            rel_path,
            sym.start_line,
            enclosing_test.as_deref(),
            &mut call_sites,
        );
    }

    let function_cfgs = if build_cfgs {
        build_function_cfgs(&tree, source.as_bytes(), &symbols, lang)
    } else {
        Vec::new()
    };

    Some(FileGraphData {
        path: rel_path.to_string(),
        language: lang,
        symbols,
        imports,
        call_sites,
        function_cfgs,
    })
}

/// Absorb one `FileGraphData` into the graph: emit File/Symbol/CallSite
/// nodes and ExitsVia edges, then drop `data` so its `FunctionCfg`s and
/// call-site metadata are freed before the next message arrives.
///
/// Cross-file edges (Imports, Calls) are queued in `deferred_imports` and
/// `deferred_calls` because they need every Symbol to be registered first.
fn absorb_file_data(
    graph: &mut CodeGraph,
    data: FileGraphData,
    deferred_imports: &mut Vec<DeferredImport>,
    deferred_calls: &mut Vec<DeferredCall>,
) {
    let FileGraphData {
        path,
        language,
        symbols,
        imports,
        call_sites,
        function_cfgs,
    } = data;

    // 5a: File node
    let path_spur = graph.symbols.intern(&path);
    let file_idx = graph.graph.add_node(NodeWeight::File {
        path: path_spur,
        language,
    });
    graph.file_nodes.insert(path_spur, file_idx);

    // 5b: Symbol nodes + DefinedIn/Contains/Exports
    for sym in &symbols {
        let sym_file_spur = graph.symbols.intern(&sym.file_path);
        let sym_name_spur = graph.symbols.intern(&sym.name);
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: sym_name_spur,
            kind: sym.kind,
            file_path: sym_file_spur,
            start_line: sym.start_line,
            end_line: sym.end_line,
            exported: sym.is_exported,
        });
        graph
            .graph
            .add_edge(sym_idx, file_idx, EdgeWeight::DefinedIn);
        graph
            .graph
            .add_edge(file_idx, sym_idx, EdgeWeight::Contains);
        if sym.is_exported {
            graph.graph.add_edge(file_idx, sym_idx, EdgeWeight::Exports);
        }
        graph
            .symbol_nodes
            .insert((sym_file_spur, sym.start_line), sym_idx);
        graph
            .symbols_by_name
            .entry(sym_name_spur)
            .or_default()
            .push(sym_idx);
    }

    // 5c: queue imports for cross-file resolution. Also stash them on the
    // graph so the Cozo writer can persist raw_imports for incremental
    // refresh (issue 08).
    graph
        .raw_imports
        .entry(path.clone())
        .or_default()
        .extend(imports.iter().cloned());
    for import in imports {
        deferred_imports.push(DeferredImport {
            from_file_path: path.clone(),
            language,
            import,
        });
    }

    // 5d: CallSite nodes + Contains edges. Calls edges are deferred until
    // every Symbol is registered.
    for cs in call_sites {
        let caller_file_spur = graph.symbols.intern(&cs.caller_file);
        let caller_key = (caller_file_spur, cs.caller_symbol_line);
        let caller_idx = graph.symbol_nodes.get(&caller_key).copied();

        let callee_spur = graph.symbols.intern(&cs.callee_name);
        let arg_literal_spurs: Option<Box<[Spur]>> = if cs.arg_literals.is_empty() {
            None
        } else {
            Some(
                cs.arg_literals
                    .iter()
                    .map(|s| graph.symbols.intern(s))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            )
        };
        let enclosing_spur = cs
            .enclosing_test_name
            .as_deref()
            .map(|s| graph.symbols.intern(s));
        let callsite_idx = graph.graph.add_node(NodeWeight::CallSite {
            name: callee_spur,
            file_path: caller_file_spur,
            line: cs.line,
            arg_literals: arg_literal_spurs,
            enclosing_test_name: enclosing_spur,
            caller_symbol: caller_idx,
        });
        if let Some(caller_idx) = caller_idx {
            graph
                .graph
                .add_edge(caller_idx, callsite_idx, EdgeWeight::Contains);
            deferred_calls.push(DeferredCall {
                caller_idx,
                callee_name: cs.callee_name,
            });
        }
    }

    // 5e: ExitsVia edges from CFGs. CFGs are consumed here and dropped.
    for (start_line, cfg) in function_cfgs {
        let Some(&sym_idx) = graph.symbol_nodes.get(&(path_spur, start_line)) else {
            continue;
        };
        graph.function_cfg_indices.insert(sym_idx);
        let function_name_spur = match &graph.graph[sym_idx] {
            NodeWeight::Symbol { name, .. } => *name,
            _ => continue,
        };
        for &exit_block in &cfg.exits {
            let (exit_kind, exit_label) = classify_cfg_exit(&cfg, exit_block);
            let exit_label_spur = exit_label.as_deref().map(|s| graph.symbols.intern(s));
            let exit_line = cfg.blocks[exit_block]
                .statements
                .last()
                .map(|s| s.line)
                .unwrap_or(start_line);
            let exit_idx = graph.graph.add_node(NodeWeight::CfgExit {
                function_node: sym_idx,
                function_name: function_name_spur,
                file_path: path_spur,
                line: exit_line,
                exit_kind: exit_kind.clone(),
                exit_label: exit_label_spur,
            });
            graph
                .graph
                .add_edge(sym_idx, exit_idx, EdgeWeight::Contains);
            graph
                .graph
                .add_edge(sym_idx, exit_idx, EdgeWeight::ExitsVia(exit_kind));
        }
    }
}

/// Resolve an import to a file path string (unwrapping GraphNode).
fn resolve_import_to_file(
    source_file: &str,
    import: &ImportInfo,
    language: Language,
    known_files: &HashSet<String>,
) -> Option<String> {
    use crate::graph::GraphNode;
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
    let builder = match crate::languages::cfg::cfg_builder_for_language(language) {
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

/// Classify how a function reaches `exit_block`. The kind comes from the
/// edge weight on the (single) inbound edge — when an exit block has multiple
/// inbound edges of different kinds, kinds are picked in priority order
/// Exception > Cleanup > TrueBranch > FalseBranch > Normal.
///
/// `exit_label`:
/// - branches: `condition_vars.join(" & ")` from the originating Guard, if any.
/// - exception: callee name of any `Call` in the exit block.
/// - normal/cleanup: `None`.
fn classify_cfg_exit(
    cfg: &FunctionCfg,
    exit_block: petgraph::graph::NodeIndex,
) -> (CfgExitKind, Option<String>) {
    use petgraph::Direction;
    use petgraph::visit::EdgeRef;

    // Pick the highest-priority inbound edge kind.
    let mut best: Option<CfgExitKind> = None;
    for edge in cfg.blocks.edges_directed(exit_block, Direction::Incoming) {
        let kind = CfgExitKind::from_cfg_edge(edge.weight());
        best = Some(match (best.take(), kind) {
            (None, k) => k,
            (Some(prev), k) => higher_priority(prev, k),
        });
    }
    let kind = best.unwrap_or(CfgExitKind::Normal);

    let label = match &kind {
        CfgExitKind::TrueBranch | CfgExitKind::FalseBranch => {
            // Walk inbound edges, find the predecessor block whose Guard
            // statement preceded this branch, and join its condition_vars.
            let mut label = None;
            for edge in cfg.blocks.edges_directed(exit_block, Direction::Incoming) {
                let pred = edge.source();
                for stmt in &cfg.blocks[pred].statements {
                    if let CfgStatementKind::Guard { condition_vars } = &stmt.kind
                        && !condition_vars.is_empty()
                    {
                        label = Some(condition_vars.join(" & "));
                    }
                }
            }
            label
        }
        CfgExitKind::Exception => {
            cfg.blocks[exit_block]
                .statements
                .iter()
                .find_map(|s| match &s.kind {
                    CfgStatementKind::Call { name, .. } => Some(name.clone()),
                    _ => None,
                })
        }
        _ => None,
    };

    (kind, label)
}

fn higher_priority(a: CfgExitKind, b: CfgExitKind) -> CfgExitKind {
    fn rank(k: &CfgExitKind) -> u8 {
        match k {
            CfgExitKind::Exception => 4,
            CfgExitKind::Cleanup => 3,
            CfgExitKind::TrueBranch => 2,
            CfgExitKind::FalseBranch => 1,
            CfgExitKind::Normal => 0,
        }
    }
    if rank(&b) > rank(&a) { b } else { a }
}

/// Find a tree-sitter node that matches the given line range.
pub(crate) fn find_node_at_line(
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
    enclosing_test_name: Option<&str>,
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
        let line = node.start_position().row as u32 + 1;
        let arg_literals = extract_call_args(node, source);
        out.push(CallSiteData {
            callee_name: name,
            caller_file: file_path.to_string(),
            caller_symbol_line,
            line,
            arg_literals,
            enclosing_test_name: enclosing_test_name.map(|s| s.to_string()),
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
            enclosing_test_name,
            out,
        );
    }
}

/// Extract literal arguments (strings/numbers/bools) from a call expression node.
/// Non-literal arguments are skipped. The list of node kinds matched is the union
/// across all 10 supported tree-sitter grammars; per-language refinement is a
/// follow-up.
fn extract_call_args(node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let args_node = match node
        .child_by_field_name("arguments")
        .or_else(|| node.child_by_field_name("argument_list"))
    {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        if !is_literal_kind(child.kind()) {
            continue;
        }
        if let Ok(text) = child.utf8_text(source) {
            out.push(strip_quotes(text).to_string());
        }
    }
    out
}

fn is_literal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "string_literal"
            | "interpreted_string_literal"
            | "raw_string_literal"
            | "number"
            | "number_literal"
            | "integer"
            | "integer_literal"
            | "float"
            | "float_literal"
            | "decimal_integer_literal"
            | "decimal_floating_point_literal"
            | "true"
            | "false"
            | "boolean"
            | "boolean_literal"
            | "true_lit"
            | "false_lit"
            | "null"
            | "null_literal"
            | "nil"
            | "none"
            | "encapsed_string"
            | "shell_command_string"
    )
}

fn strip_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'' || first == b'`') && last == first {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Heuristic: does this symbol name look like a test function?
fn is_test_function_name(name: &str) -> bool {
    if name.starts_with("test_") || name.starts_with("Test") || name.ends_with("_test") {
        return true;
    }
    if name.ends_with("Test") && name.len() > 4 {
        return true;
    }
    if name.starts_with("it ") || name.starts_with("describe ") {
        return true;
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::workspace::Workspace;

    fn build_graph(dir: &std::path::Path, langs: &[Language]) -> CodeGraph {
        let ws = Workspace::load(dir, langs, None).unwrap();
        GraphBuilder::new(&ws, langs).build().unwrap()
    }

    fn collect_callsites(graph: &CodeGraph) -> Vec<&NodeWeight> {
        graph
            .graph
            .node_weights()
            .filter(|nw| matches!(nw, NodeWeight::CallSite { .. }))
            .collect()
    }

    #[test]
    fn test_callsite_node_emitted_per_call_expression() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            r#"fn foo() { bar(); baz(); qux(); }
fn bar() {}
fn baz() {}
fn qux() {}
"#,
        )
        .unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        let callsites = collect_callsites(&graph);
        // foo calls bar/baz/qux — three call expressions inside foo.
        assert!(
            callsites.len() >= 3,
            "expected >= 3 callsites, got {}",
            callsites.len()
        );
    }

    #[test]
    fn test_callsite_arg_literals_strings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            r#"fn foo() { bar("hello", 42); }
fn bar(_a: &str, _b: i32) {}
"#,
        )
        .unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        let bar_call = graph
            .graph
            .node_weights()
            .find_map(|nw| match nw {
                NodeWeight::CallSite {
                    name, arg_literals, ..
                } if graph.symbols.resolve(*name) == "bar" => Some(arg_literals.clone()),
                _ => None,
            })
            .expect("bar callsite");
        let slice = bar_call.expect("bar should have arg literals");
        let resolved: Vec<&str> = slice.iter().map(|s| graph.symbols.resolve(*s)).collect();
        assert!(
            resolved.contains(&"hello"),
            "expected hello in {:?}",
            resolved
        );
        assert!(resolved.iter().any(|s| *s == "42"), "expected 42 in {:?}", resolved);
    }

    #[test]
    fn test_callsite_skips_non_literal_args() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            r#"fn foo(x: i32, y: i32) { bar(x + y); }
fn bar(_z: i32) {}
"#,
        )
        .unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        let bar_call = graph
            .graph
            .node_weights()
            .find_map(|nw| match nw {
                NodeWeight::CallSite {
                    name, arg_literals, ..
                } if graph.symbols.resolve(*name) == "bar" => Some(arg_literals.clone()),
                _ => None,
            })
            .expect("bar callsite");
        assert!(
            bar_call.is_none(),
            "expected no literals, got {:?}",
            bar_call
        );
    }

    #[test]
    fn test_callsite_enclosing_test_name_python() {
        let dir = tempfile::tempdir().unwrap();
        let tests = dir.path().join("tests");
        std::fs::create_dir_all(&tests).unwrap();
        std::fs::write(
            tests.join("test_login.py"),
            r#"def test_login():
    user_login("alice")

def user_login(name):
    pass
"#,
        )
        .unwrap();
        let graph = build_graph(dir.path(), &[Language::Python]);
        let cs = graph
            .graph
            .node_weights()
            .find_map(|nw| match nw {
                NodeWeight::CallSite {
                    name,
                    enclosing_test_name,
                    ..
                } if graph.symbols.resolve(*name) == "user_login" => {
                    Some(enclosing_test_name.map(|s| graph.symbols.resolve(s).to_string()))
                }
                _ => None,
            })
            .expect("user_login callsite");
        assert_eq!(cs.as_deref(), Some("test_login"));
    }

    fn collect_cfg_exits(graph: &CodeGraph) -> Vec<&NodeWeight> {
        graph
            .graph
            .node_weights()
            .filter(|nw| matches!(nw, NodeWeight::CfgExit { .. }))
            .collect()
    }

    #[test]
    fn test_select_cfg_exit_normal_return_rust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn foo() { let _ = 1; }\n").unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        let exits = collect_cfg_exits(&graph);
        assert!(!exits.is_empty(), "expected at least one CfgExit node");
        assert!(exits.iter().any(|nw| matches!(
            nw,
            NodeWeight::CfgExit {
                exit_kind: CfgExitKind::Normal,
                ..
            }
        )));
    }

    #[test]
    fn test_select_cfg_exit_no_cfg_for_non_function_symbol() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "struct Foo;\n").unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        // No function -> no CfgExit nodes.
        assert!(collect_cfg_exits(&graph).is_empty());
    }

    #[test]
    fn test_exits_via_edge_emitted_for_each_exit() {
        use petgraph::Direction;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn foo() { let _ = 1; }\n").unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);

        // Find the function symbol and assert it has at least one outgoing
        // ExitsVia edge.
        let foo_idx = graph
            .graph
            .node_indices()
            .find(|&i| match &graph.graph[i] {
                NodeWeight::Symbol { name, .. } => graph.symbols.resolve(*name) == "foo",
                _ => false,
            })
            .expect("foo symbol");
        let count = graph
            .graph
            .edges_directed(foo_idx, Direction::Outgoing)
            .filter(|e| matches!(e.weight(), EdgeWeight::ExitsVia(_)))
            .count();
        assert!(count >= 1, "expected ExitsVia edge from foo");
    }

    #[test]
    fn test_callsite_enclosing_test_name_none_in_prod_code() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("prod.py"),
            r#"def login(name):
    user_login(name)

def user_login(name):
    pass
"#,
        )
        .unwrap();
        let graph = build_graph(dir.path(), &[Language::Python]);
        let cs = graph
            .graph
            .node_weights()
            .find_map(|nw| match nw {
                NodeWeight::CallSite {
                    name,
                    enclosing_test_name,
                    ..
                } if graph.symbols.resolve(*name) == "user_login" => {
                    Some(enclosing_test_name.map(|s| graph.symbols.resolve(s).to_string()))
                }
                _ => None,
            })
            .expect("user_login callsite");
        assert_eq!(cs, None);
    }
}
