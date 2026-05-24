use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use anyhow::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, info, info_span};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use tree_sitter::Query;

use crate::cozo::{CozoStore, CozoWriter};
use crate::language::Language;
use crate::languages;
use crate::models::{
    AttrsBucket, CommentInfo, FieldTypeRow, ImportInfo, InheritanceRow, ParameterTypeRow,
    ReferencesBucket, ReturnsTypeRow, SymbolInfo, SymbolKind, ThrowsRow, TypeRow,
};
use crate::parser;
use crate::storage::workspace::Workspace;

use super::{CodeGraph, EdgeWeight, NodeIndex, NodeWeight, Spur};

/// Flush the streaming writer every this many files. Caps peak writer
/// memory to roughly N files' worth of per-file rows (refs/attrs/
/// raw_imports). Picked to amortise Cozo transaction overhead — too low
/// thrashes SQLite, too high defeats streaming.
const STREAM_FLUSH_EVERY_N_FILES: u32 = 200;

/// Per-file extraction result, collected in parallel.
struct FileGraphData {
    path: String,
    language: Language,
    symbols: Vec<SymbolInfo>,
    comments: Vec<CommentInfo>,
    imports: Vec<ImportInfo>,
    call_sites: Vec<CallSiteData>,
    /// Issue #13: type/parameter/return/inheritance rows from the
    /// per-language type extractor. Empty for languages whose extractor
    /// hasn't been wired yet.
    types: Vec<TypeRow>,
    param_types: Vec<ParameterTypeRow>,
    returns_types: Vec<ReturnsTypeRow>,
    inheritance: Vec<InheritanceRow>,
    /// Issue #14: typed field/property declarations.
    field_types: Vec<FieldTypeRow>,
    /// Issue #13 followup: declared/observed `throws` rows.
    throws: Vec<ThrowsRow>,
    /// Issue #15: per-language attribute rows. Only this file's
    /// language bucket is populated.
    attrs: AttrsBucket,
    /// Issue #16: occurrence/scope/binding facts for the resolver.
    references: ReferencesBucket,
}

/// A call site extracted from within a symbol's line range.
struct CallSiteData {
    callee_name: String,
    caller_file: String,
    caller_symbol_line: u32,
    /// Line of the call expression itself (1-based).
    line: u32,
    start_byte: u32,
    end_byte: u32,
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

/// A Calls-edge deferred until all Symbol nodes and cross-file edges are
/// present. Resolution is scoped to the caller's file: same-file symbols
/// always match; symbols in other files match only if the caller's file
/// imports that file and the callee symbol is exported.
struct DeferredCall {
    caller_idx: NodeIndex,
    caller_file_spur: Spur,
    callee_spur: Spur,
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

    pub fn build(&self, store: &CozoStore) -> Result<CodeGraph> {
        let total_files = self.workspace.file_count();
        info!(
            files = total_files,
            languages = self.languages.len(),
            "graph build starting"
        );
        // Step 1: Pre-compile queries — only for languages actually
        // present in the workspace. Each tree-sitter Query carries the
        // grammar tables; lazy-compiling avoids the ~10-30 MB cost per
        // unused language for single-lang projects.
        let mut present_langs: Vec<Language> = self
            .workspace
            .files()
            .iter()
            .filter_map(|p| self.workspace.file_language(p))
            .filter(|l| self.languages.contains(l))
            .collect();
        present_langs.sort_by_key(|l| l.as_str());
        present_langs.dedup();
        info!(
            languages_loaded = present_langs.len(),
            languages_requested = self.languages.len(),
            "compiling grammars for languages actually present"
        );

        let mut symbol_queries: HashMap<Language, Arc<Query>> = HashMap::new();
        let mut import_queries: HashMap<Language, Arc<Query>> = HashMap::new();
        let mut comment_queries: HashMap<Language, Arc<Query>> = HashMap::new();
        for &lang in &present_langs {
            symbol_queries.insert(lang, languages::compile_symbol_query(lang)?);
            import_queries.insert(lang, languages::compile_import_query(lang)?);
            if let Ok(q) = languages::compile_comment_query(lang) {
                comment_queries.insert(lang, q);
            }
        }
        let symbol_queries = Arc::new(symbol_queries);
        let import_queries = Arc::new(import_queries);
        let comment_queries = Arc::new(comment_queries);

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
        // a slow drainer accumulate the whole workspace in the queue.
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
        // Per-file name lookups built during absorption. Used by the
        // import-scoped Calls-edge resolver below.
        let mut file_symbols_by_name: HashMap<(Spur, Spur), Vec<NodeIndex>> = HashMap::new();
        let mut file_exports_by_name: HashMap<(Spur, Spur), Vec<NodeIndex>> = HashMap::new();
        // Streaming writer for per-file buckets that have no cross-file
        // dependency (references/attrs/raw_imports). Emitted to Cozo
        // during absorb and never stashed on the graph.
        let mut stream_writer = CozoWriter::new();
        let mut files_since_flush: u32 = 0;

        let workspace = self.workspace;
        let sym_q = Arc::clone(&symbol_queries);
        let imp_q = Arc::clone(&import_queries);
        let com_q = Arc::clone(&comment_queries);
        let grouped_files_ref = &grouped_files;

        let parsed = AtomicU64::new(0);
        let absorbed_files = AtomicU64::new(0);
        let target_files = grouped_files_ref.len() as u64;

        {
            let span = info_span!("graph.parse_absorb", files = target_files);
            span.pb_set_length(target_files);
            span.pb_set_style(
                &indicatif::ProgressStyle::with_template(
                    "{span_child_prefix}{spinner} parse+absorb [{bar:30}] {pos}/{len} ({eta})",
                )
                .unwrap()
                .progress_chars("=> "),
            );
            let _enter = span.enter();
            thread::scope(|s| -> Result<()> {
                let parallelism = thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4);
                let channel_bound = (parallelism * 2).max(4);
                let (tx, rx) = mpsc::sync_channel::<FileGraphData>(channel_bound);

                let parsed_ref = &parsed;
                s.spawn(move || {
                    pool.install(|| {
                        grouped_files_ref
                            .par_iter()
                            .for_each_with(tx, |tx, &(lang, rel_path)| {
                                if let Some(data) = parse_one_file(
                                    lang, rel_path, workspace, &sym_q, &imp_q, &com_q,
                                ) {
                                    let n = parsed_ref.fetch_add(1, Ordering::Relaxed) + 1;
                                    if n.is_multiple_of(500) {
                                        debug!(parsed = n, of = target_files, "parsing progress");
                                    }
                                    let _ = tx.send(data);
                                }
                            });
                    });
                });

                while let Ok(data) = rx.recv() {
                    let path = data.path.clone();
                    absorb_file_data(
                        &mut graph,
                        data,
                        &mut deferred_imports,
                        &mut deferred_calls,
                        &mut file_symbols_by_name,
                        &mut file_exports_by_name,
                        &mut stream_writer,
                    );
                    files_since_flush += 1;
                    if files_since_flush >= STREAM_FLUSH_EVERY_N_FILES {
                        stream_writer.flush(store)?;
                        files_since_flush = 0;
                    }
                    tracing::Span::current().pb_inc(1);
                    let n = absorbed_files.fetch_add(1, Ordering::Relaxed) + 1;
                    if n.is_multiple_of(500) {
                        debug!(absorbed = n, of = target_files, last = %path, "absorption progress");
                    }
                }

                Ok(())
            })?;
            // Flush the streaming writer's tail rows before cross-file
            // resolution runs — keeps populate's later phases from
            // racing with leftover per-file rows.
            stream_writer.flush(store)?;
            info!(
                parsed = parsed.load(Ordering::Relaxed),
                absorbed = absorbed_files.load(Ordering::Relaxed),
                deferred_imports = deferred_imports.len(),
                deferred_calls = deferred_calls.len(),
                "parse + absorb done"
            );
        }

        let _resolve_span = info_span!("graph.resolve_refs").entered();

        // Resolve deferred imports now that every File node exists. Track
        // each resolved import in `file_imports` so the call resolver below
        // can scope name lookups to the caller's import set.
        let mut file_imports: HashMap<Spur, Vec<Spur>> = HashMap::new();
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
                graph.add_edge(from_file_idx, to_file_idx, EdgeWeight::Imports);
                file_imports.entry(from_spur).or_default().push(to_spur);
            }
        }

        // Resolve deferred Calls edges with import-scoped name lookup:
        //
        //   - Same-file: any symbol in the caller's file with a matching name.
        //   - Cross-file: only symbols whose file the caller imports, and
        //     which are themselves exported.
        //
        // Drops the global name-collision noise that the old resolver
        // produced (one Calls edge per same-named symbol anywhere in the
        // workspace). Method calls and otherwise-unresolved names simply
        // don't get a cross-file edge — that's the intended behaviour.
        let mut targets: Vec<NodeIndex> = Vec::new();
        for dc in deferred_calls {
            targets.clear();

            if let Some(syms) = file_symbols_by_name.get(&(dc.caller_file_spur, dc.callee_spur)) {
                targets.extend(syms.iter().copied());
            }

            if let Some(imp_files) = file_imports.get(&dc.caller_file_spur) {
                for &imp_file in imp_files {
                    if let Some(syms) = file_exports_by_name.get(&(imp_file, dc.callee_spur)) {
                        targets.extend(syms.iter().copied());
                    }
                }
            }

            // Filter out non-callable targets (parameters, locals, types) —
            // a name match on a parameter or `let` binding is never a real
            // call edge. Constants are kept because some Rust constants
            // can be function pointers, but the heuristic stays
            // intentionally conservative.
            targets.retain(|&idx| {
                matches!(
                    &graph.nodes[idx],
                    NodeWeight::Symbol {
                        kind: SymbolKind::Function
                            | SymbolKind::Method
                            | SymbolKind::ArrowFunction
                            | SymbolKind::Macro,
                        ..
                    }
                )
            });
            targets.sort_unstable();
            targets.dedup();
            for &target_idx in &targets {
                if target_idx != dc.caller_idx {
                    graph.add_edge(dc.caller_idx, target_idx, EdgeWeight::Calls);
                }
            }
        }

        let edge_count: usize = graph.out_edges.iter().map(|v| v.len()).sum();
        info!(
            nodes = graph.nodes.len(),
            edges = edge_count,
            "graph build complete"
        );
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
    comment_queries: &HashMap<Language, Arc<Query>>,
) -> Option<FileGraphData> {
    let sym_query = symbol_queries.get(&lang)?;
    let imp_query = import_queries.get(&lang)?;

    let mut ts_parser = parser::create_parser(lang).ok()?;
    let source = workspace.read_file(rel_path)?;
    let tree = ts_parser.parse(&*source, None)?;

    let symbols = languages::extract_symbols(&tree, source.as_bytes(), sym_query, rel_path, lang);
    let imports = languages::extract_imports(&tree, source.as_bytes(), imp_query, rel_path, lang);
    let comments = if let Some(cq) = comment_queries.get(&lang) {
        languages::extract_comments(&tree, source.as_bytes(), cq, rel_path, lang)
    } else {
        Vec::new()
    };

    let call_node_types = call_expression_types(lang);
    let is_test = crate::classify::is_test_file(rel_path);
    let mut call_sites = Vec::new();
    for sym in &symbols {
        // Only function-like symbols can be call-site owners. Parameters
        // and locals never enclose calls (or shouldn't, semantically) —
        // attributing calls to them creates phantom caller_ids that
        // explode the calls relation.
        if !matches!(
            sym.kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::ArrowFunction
                | SymbolKind::Macro
        ) {
            continue;
        }
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

    // Issue #13 + #14: per-language type / inheritance / field-type
    // extraction. Languages without typed fields leave field_types
    // empty.
    let (types, param_types, returns_types, inheritance, field_types) =
        languages::extract_types(&tree, source.as_bytes(), rel_path, lang);

    // Issue #13 followup: per-language `throws` extraction (Java/C#/PHP).
    let throws = languages::extract_throws(&tree, source.as_bytes(), rel_path, lang);

    // Issue #15: per-language attribute extraction.
    let attrs = languages::extract_attrs(&tree, source.as_bytes(), rel_path, lang, &symbols);

    // Issue #16: occurrence/scope/binding fact emission.
    let references =
        languages::extract_references(&tree, source.as_bytes(), rel_path, lang, &symbols);

    Some(FileGraphData {
        path: rel_path.to_string(),
        language: lang,
        symbols,
        comments,
        imports,
        call_sites,
        types,
        param_types,
        returns_types,
        inheritance,
        field_types,
        throws,
        attrs,
        references,
    })
}

#[allow(clippy::too_many_arguments)]
fn absorb_file_data(
    graph: &mut CodeGraph,
    data: FileGraphData,
    deferred_imports: &mut Vec<DeferredImport>,
    deferred_calls: &mut Vec<DeferredCall>,
    file_symbols_by_name: &mut HashMap<(Spur, Spur), Vec<NodeIndex>>,
    file_exports_by_name: &mut HashMap<(Spur, Spur), Vec<NodeIndex>>,
    stream_writer: &mut CozoWriter,
) {
    let FileGraphData {
        path,
        language,
        symbols,
        comments,
        imports,
        call_sites,
        types,
        param_types,
        returns_types,
        inheritance,
        field_types,
        throws,
        attrs,
        references,
    } = data;

    // File node
    let path_spur = graph.symbols.intern(&path);
    let file_idx = graph.add_node(NodeWeight::File {
        path: path_spur,
        language,
    });
    graph.file_nodes.insert(path_spur, file_idx);

    // Pass 1: allocate Symbol nodes in original extraction order. This
    // preserves the baseline behaviour where `symbol_nodes[(f, line)]`
    // resolves to whichever same-line symbol the extractor emitted *last*
    // — existing call-site attribution and baseline snapshot counts
    // depend on that mapping. `qualified_name` defaults to the leaf name
    // here; pass 2 rewrites it with the parent-chain join.
    let mut sym_indices: Vec<NodeIndex> = Vec::with_capacity(symbols.len());
    for sym in &symbols {
        let sym_file_spur = graph.symbols.intern(&sym.file_path);
        let sym_name_spur = graph.symbols.intern(&sym.name);
        let sym_idx = graph.add_node(NodeWeight::Symbol {
            name: sym_name_spur,
            qualified_name: sym_name_spur,
            kind: sym.kind,
            file_path: sym_file_spur,
            start_byte: sym.start_byte,
            end_byte: sym.end_byte,
            start_line: sym.start_line,
            start_col: sym.start_column,
            end_line: sym.end_line,
            end_col: sym.end_column,
            exported: sym.is_exported,
            visibility: sym.visibility,
            is_async: sym.is_async,
            is_static: sym.is_static,
            is_abstract: sym.is_abstract,
            is_mutable: sym.is_mutable,
        });
        graph.add_edge(sym_idx, file_idx, EdgeWeight::DefinedIn);
        graph.add_edge(file_idx, sym_idx, EdgeWeight::Contains);
        if sym.is_exported {
            graph.add_edge(file_idx, sym_idx, EdgeWeight::Exports);
        }
        graph
            .symbol_nodes
            .insert((sym_file_spur, sym.start_line), sym_idx);
        graph
            .symbols_by_name
            .entry(sym_name_spur)
            .or_default()
            .push(sym_idx);
        file_symbols_by_name
            .entry((sym_file_spur, sym_name_spur))
            .or_default()
            .push(sym_idx);
        if sym.is_exported {
            file_exports_by_name
                .entry((sym_file_spur, sym_name_spur))
                .or_default()
                .push(sym_idx);
        }
        sym_indices.push(sym_idx);
    }

    // Pass 2: derive parent-symbol containment via byte ranges and
    // compute `qualified_name` from the parent chain.
    //
    // Walk symbols in outer-first order (start_byte ASC, end_byte DESC
    // tie-break) so an enclosing scope is seen before any nested symbol.
    // Maintain a stack of currently-open containers; the innermost open
    // one whose range covers the current symbol is its parent.
    let mut order: Vec<usize> = (0..symbols.len()).collect();
    order.sort_by(|&a, &b| {
        symbols[a]
            .start_byte
            .cmp(&symbols[b].start_byte)
            .then_with(|| symbols[b].end_byte.cmp(&symbols[a].end_byte))
    });
    let sep = languages::qname_separator(language);
    let mut open: Vec<(usize, u32)> = Vec::new();
    let mut parent_of: Vec<Option<usize>> = vec![None; symbols.len()];
    for &i in &order {
        let sym = &symbols[i];
        while let Some(&(_, end)) = open.last() {
            if end <= sym.start_byte {
                open.pop();
            } else {
                break;
            }
        }
        parent_of[i] = open
            .iter()
            .rev()
            .find_map(|&(idx, end)| if sym.end_byte <= end { Some(idx) } else { None });
        open.push((i, sym.end_byte));
    }

    // Compute qualified_name in outer-first order and add Symbol→Symbol
    // Contains edges between each child and its parent.
    let mut qnames: Vec<String> = vec![String::new(); symbols.len()];
    for &i in &order {
        let sym = &symbols[i];
        qnames[i] = match parent_of[i] {
            Some(p) => format!("{}{}{}", &qnames[p], sep, sym.name),
            None => sym.name.clone(),
        };
        if let Some(p) = parent_of[i] {
            graph.add_edge(sym_indices[p], sym_indices[i], EdgeWeight::Contains);
        }
    }

    // Patch each symbol node's qualified_name spur.
    for (i, qname) in qnames.into_iter().enumerate() {
        let qname_spur = graph.symbols.intern(&qname);
        if let NodeWeight::Symbol { qualified_name, .. } = &mut graph.nodes[sym_indices[i]] {
            *qualified_name = qname_spur;
        }
    }

    // Stash comments per file so the Cozo writer can emit `comment` rows.
    if !comments.is_empty() {
        graph
            .comments
            .entry(path.clone())
            .or_default()
            .extend(comments);
    }

    // Queue imports for cross-file resolution AND stream the raw_import
    // rows directly to Cozo — they're per-file with no cross-file
    // dependency, so we don't need to keep them on the graph for the
    // populate phase to walk later (issue 08 incremental-refresh path
    // reads from Cozo).
    let lang_str = language.as_str();
    for (idx, import) in imports.iter().enumerate() {
        stream_writer.push_raw_import(
            &path,
            idx as i64,
            &import.module_specifier,
            lang_str,
            &import.kind,
        );
    }
    for import in imports {
        deferred_imports.push(DeferredImport {
            from_file_path: path.clone(),
            language,
            import,
        });
    }

    // Issue #13: stash per-file type / signature / inheritance rows so
    // `from_code_graph` can emit them. Empty vectors carry no cost; we
    // only insert when something is actually there.
    if !types.is_empty() {
        graph.types.entry(path.clone()).or_default().extend(types);
    }
    if !param_types.is_empty() {
        graph
            .param_types
            .entry(path.clone())
            .or_default()
            .extend(param_types);
    }
    if !returns_types.is_empty() {
        graph
            .returns_types
            .entry(path.clone())
            .or_default()
            .extend(returns_types);
    }
    if !inheritance.is_empty() {
        graph
            .inheritance
            .entry(path.clone())
            .or_default()
            .extend(inheritance);
    }
    if !field_types.is_empty() {
        graph
            .field_types
            .entry(path.clone())
            .or_default()
            .extend(field_types);
    }
    if !throws.is_empty() {
        graph.throws.entry(path.clone()).or_default().extend(throws);
    }
    // Issue #15: stream per-language attrs directly to Cozo. Each row
    // carries its own symbol_id, so no cross-file resolution is needed.
    for r in &attrs.rust {
        stream_writer.push_rust_attrs(&r.symbol_id, r.is_unsafe, r.is_const, &r.derives);
    }
    for r in &attrs.python {
        stream_writer.push_python_attrs(
            &r.symbol_id,
            &r.decorators,
            r.is_generator,
            r.is_coroutine,
            r.docstring_style.as_deref(),
        );
    }
    for r in &attrs.typescript {
        stream_writer.push_typescript_attrs(
            &r.symbol_id,
            r.is_readonly,
            r.is_optional,
            &r.type_parameters,
        );
    }
    for r in &attrs.cpp {
        stream_writer.push_cpp_attrs(
            &r.symbol_id,
            r.is_virtual,
            r.is_const,
            r.is_noexcept,
            r.is_template,
            r.is_constexpr,
            r.is_override,
            r.is_final,
        );
    }
    for r in &attrs.csharp {
        stream_writer.push_csharp_attrs(&r.symbol_id, &r.attributes, r.is_partial, r.is_sealed);
    }
    for r in &attrs.go {
        stream_writer.push_go_attrs(&r.symbol_id, r.is_exported, r.has_receiver, &r.build_tags);
    }
    for r in &attrs.php {
        stream_writer.push_php_attrs(&r.symbol_id, r.is_final, &r.uses_traits, &r.attributes);
    }
    for r in &attrs.c {
        stream_writer.push_c_attrs(
            &r.symbol_id,
            r.is_file_static,
            r.is_extern,
            r.is_inline,
            r.is_const,
            r.is_volatile,
            r.is_restrict,
            &r.gcc_attributes,
        );
    }
    for r in &attrs.java {
        stream_writer.push_java_attrs(
            &r.symbol_id,
            &r.annotations,
            r.is_final,
            r.is_synchronized,
            &r.throws_clause,
        );
    }
    // Issue #16: stream occurrence/scope/binding facts directly. Each
    // row is self-contained (ids are file-local) and the resolver reads
    // them from Cozo anyway, so there's no reason to keep them on graph.
    for r in &references.scopes {
        stream_writer.push_scope(
            &r.id,
            r.parent_id.as_deref(),
            &r.file_path,
            &r.kind,
            r.start_byte as i64,
            r.end_byte as i64,
        );
    }
    for r in &references.bindings {
        stream_writer.push_binding(
            &r.scope_id,
            &r.name,
            r.start_byte as i64,
            r.symbol_id.as_deref(),
            &r.binding_kind,
        );
    }
    for r in &references.occurrences {
        stream_writer.push_occurrence(
            &r.id,
            &r.name,
            &r.file_path,
            r.start_byte as i64,
            r.end_byte as i64,
            r.enclosing_symbol_id.as_deref(),
            &r.enclosing_scope_id,
            &r.occurrence_kind,
        );
    }

    // CallSite nodes + Contains edges. Calls edges are deferred until
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
        let callsite_idx = graph.add_node(NodeWeight::CallSite {
            name: callee_spur,
            file_path: caller_file_spur,
            line: cs.line,
            start_byte: cs.start_byte,
            end_byte: cs.end_byte,
            arg_literals: arg_literal_spurs,
            enclosing_test_name: enclosing_spur,
            caller_symbol: caller_idx,
        });
        if let Some(caller_idx) = caller_idx {
            graph.add_edge(caller_idx, callsite_idx, EdgeWeight::Contains);
            deferred_calls.push(DeferredCall {
                caller_idx,
                caller_file_spur,
                callee_spur,
            });
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

/// Find a tree-sitter node that matches the given line range. Used by
/// `complexity_hotspots` for on-demand metric computation.
pub fn find_node_at_line(
    node: tree_sitter::Node,
    start_line: u32,
    end_line: u32,
) -> Option<tree_sitter::Node> {
    let node_start = node.start_position().row as u32 + 1;
    let node_end = node.end_position().row as u32 + 1;

    if node_start == start_line && node_end == end_line {
        return Some(node);
    }

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
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
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
        let store = crate::cozo::CozoStore::open_in_memory().unwrap();
        GraphBuilder::new(&ws, langs).build(&store).unwrap()
    }

    fn collect_callsites(graph: &CodeGraph) -> Vec<&NodeWeight> {
        graph
            .nodes
            .iter()
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
            .nodes
            .iter()
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
        assert!(resolved.contains(&"42"), "expected 42 in {:?}", resolved);
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
            .nodes
            .iter()
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
            .nodes
            .iter()
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

    fn count_calls_edges(graph: &CodeGraph) -> usize {
        graph
            .out_edges
            .iter()
            .flat_map(|v| v.iter())
            .filter(|(_, w)| matches!(w, EdgeWeight::Calls))
            .count()
    }

    #[test]
    fn test_call_resolution_scoped_to_imports_same_name_different_files() {
        // Two files each define `init`; a third file calls `init` and
        // imports only one of them. The call must resolve to the imported
        // `init` and not the unrelated one.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn init() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn init() {}\n").unwrap();
        std::fs::write(
            dir.path().join("c.rs"),
            "use self::a::init;\nfn run() { init(); }\n",
        )
        .unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);

        // Exactly one Calls edge: c.rs::run -> a.rs::init. No b.rs::init edge.
        let run_idx = graph.find_symbols_by_name("run")[0];
        let calls_from_run: Vec<&NodeWeight> = graph.out_edges[run_idx]
            .iter()
            .filter(|(_, w)| matches!(w, EdgeWeight::Calls))
            .map(|(t, _)| &graph.nodes[*t])
            .collect();
        assert_eq!(
            calls_from_run.len(),
            1,
            "expected 1 Calls edge from run, got {}: {:?}",
            calls_from_run.len(),
            calls_from_run
        );
        let target_file = match calls_from_run[0] {
            NodeWeight::Symbol { file_path, .. } => graph.symbols.resolve(*file_path),
            _ => panic!("Calls edge target was not a Symbol"),
        };
        assert_eq!(target_file, "a.rs");
    }

    #[test]
    fn test_call_resolution_drops_cross_file_edge_without_import() {
        // Caller calls `beta` defined in another file, with no import.
        // Old global-lookup behavior would produce a Calls edge; the
        // import-scoped resolver must produce none.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn alpha() { beta(); }\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn beta() {}\n").unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        assert_eq!(
            count_calls_edges(&graph),
            0,
            "expected 0 Calls edges without an import"
        );
    }

    #[test]
    fn test_call_resolution_skips_non_exported_imported_target() {
        // The caller imports `beta`, but `beta` is not pub. With no export,
        // the cross-file edge must not be emitted.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "use self::b::beta;\nfn alpha() { beta(); }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn beta() {}\n").unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        assert_eq!(
            count_calls_edges(&graph),
            0,
            "expected 0 Calls edges when target is not exported"
        );
    }

    #[test]
    fn test_call_resolution_same_file_does_not_need_import() {
        // Two symbols in the same file: caller -> callee should connect
        // regardless of import or export status.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn caller() { callee(); }\nfn callee() {}\n",
        )
        .unwrap();
        let graph = build_graph(dir.path(), &[Language::Rust]);
        assert!(
            count_calls_edges(&graph) >= 1,
            "expected at least 1 same-file Calls edge"
        );
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
            .nodes
            .iter()
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
