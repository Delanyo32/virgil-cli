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

use crate::classify::{is_barrel_file, is_test_file};
use crate::cozo::from_code_graph::{extract_nolints, is_generated_marker, symbol_id};
use crate::cozo::{CozoStore, CozoWriter};
use crate::language::Language;
use crate::languages;
use crate::models::{
    AttrsBucket, CommentInfo, FieldTypeRow, ImportInfo, InheritanceRow, ParameterTypeRow,
    ReferencesBucket, ReturnsTypeRow, SymbolInfo, SymbolKind, ThrowsRow, TypeRow,
};
use crate::parser;
use crate::storage::workspace::Workspace;

use super::{CodeGraph, Spur};

/// Flush the streaming writer every this many files. Caps peak writer
/// memory to roughly N files' worth of in-flight rows. Picked to
/// amortise Cozo transaction overhead — too low thrashes SQLite, too
/// high defeats streaming.
const STREAM_FLUSH_EVERY_N_FILES: u32 = 200;

/// Minimal record kept in build-local lookup maps that replaces the
/// old `NodeIndex`. Carries just enough to filter call targets by kind
/// and to write `*calls` rows by id.
#[derive(Clone)]
struct AbsorbedSymbol {
    id: String,
    kind: SymbolKind,
}

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

/// A call site extracted from within a symbol's line range. After
/// Slice B the only consumer is the deferred-Calls resolver, which
/// needs caller location to write `*calls.call_site_*` columns —
/// nothing else is read off this record, so it stays minimal.
struct CallSiteData {
    callee_name: String,
    caller_file: String,
    caller_symbol_line: u32,
    start_byte: u32,
    end_byte: u32,
}

/// An import deferred until all File nodes are present.
struct DeferredImport {
    from_file_path: String,
    language: Language,
    import: ImportInfo,
}

/// A Calls-edge deferred until every file's symbol table is fully
/// populated. Resolution is scoped to the caller's file: same-file
/// symbols always match; symbols in other files match only if the
/// caller's file imports that file and the callee symbol is exported.
/// Carries the call-site location so the resolver can emit a `*calls`
/// row directly without a second lookup pass.
struct DeferredCall {
    caller_id: String,
    caller_file_spur: Spur,
    callee_spur: Spur,
    site_file: String,
    site_start_byte: u32,
    site_end_byte: u32,
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
        // import-scoped Calls resolver below. Holds slim
        // `AbsorbedSymbol`s in place of the old `NodeIndex`.
        let mut file_symbols_by_name: HashMap<(Spur, Spur), Vec<AbsorbedSymbol>> = HashMap::new();
        let mut file_exports_by_name: HashMap<(Spur, Spur), Vec<AbsorbedSymbol>> = HashMap::new();
        // Set of file-path spurs that have been absorbed. Replaces the
        // old `graph.file_nodes` map (we don't need the file's id, just
        // existence, since cross-file `Imports` rows key on path).
        let mut file_known_spurs: HashSet<Spur> = HashSet::new();
        // Streaming writer for all node + edge Cozo rows. Drained
        // periodically during absorb and once after cross-file
        // resolution so peak memory stays bounded.
        let mut stream_writer = CozoWriter::new();
        let mut files_since_flush: u32 = 0;
        // repo_id derives from the workspace root's basename. S3
        // workspaces have synthetic `s3://bucket/prefix` roots — the
        // last path segment is acceptable here. Mirrors what
        // `cozo::populate` used to derive.
        let repo_id = self
            .workspace
            .root()
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

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
                        workspace,
                        &repo_id,
                        &mut deferred_imports,
                        &mut deferred_calls,
                        &mut file_symbols_by_name,
                        &mut file_exports_by_name,
                        &mut file_known_spurs,
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

        // Resolve deferred imports now that every file has been
        // absorbed. Emit `*imports` Cozo rows directly; the in-memory
        // `file_imports` map only exists long enough for the call
        // resolver to scope its lookups.
        let mut file_imports: HashMap<Spur, Vec<Spur>> = HashMap::new();
        let mut imports_emitted: usize = 0;
        for di in deferred_imports {
            let Some(from_spur) = graph.symbols.get(&di.from_file_path) else {
                continue;
            };
            if !file_known_spurs.contains(&from_spur) {
                continue;
            }
            if let Some(resolved) =
                resolve_import_to_file(&di.from_file_path, &di.import, di.language, &known_files)
                && let Some(to_spur) = graph.symbols.get(&resolved)
                && file_known_spurs.contains(&to_spur)
                && from_spur != to_spur
            {
                stream_writer.push_imports(&di.from_file_path, &resolved);
                file_imports.entry(from_spur).or_default().push(to_spur);
                imports_emitted += 1;
            }
        }

        // Resolve deferred Calls with import-scoped name lookup:
        //
        //   - Same-file: any symbol in the caller's file with a matching name.
        //   - Cross-file: only symbols whose file the caller imports, and
        //     which are themselves exported.
        //
        // Drops the global name-collision noise that the old resolver
        // produced (one Calls row per same-named symbol anywhere in
        // the workspace). Method calls and otherwise-unresolved names
        // simply don't get a cross-file edge — that's the intended
        // behaviour. Emits `*calls` Cozo rows directly; Cozo's `:put`
        // semantics dedupe on the (caller_id, callee_id) key, matching
        // the pre-Slice-B in-graph dedup.
        let mut targets: Vec<AbsorbedSymbol> = Vec::new();
        let mut calls_emitted: usize = 0;
        for dc in deferred_calls {
            targets.clear();

            if let Some(syms) = file_symbols_by_name.get(&(dc.caller_file_spur, dc.callee_spur)) {
                targets.extend(syms.iter().cloned());
            }

            if let Some(imp_files) = file_imports.get(&dc.caller_file_spur) {
                for &imp_file in imp_files {
                    if let Some(syms) = file_exports_by_name.get(&(imp_file, dc.callee_spur)) {
                        targets.extend(syms.iter().cloned());
                    }
                }
            }

            // Filter out non-callable targets (parameters, locals, types) —
            // a name match on a parameter or `let` binding is never a real
            // call edge.
            targets.retain(|s| {
                matches!(
                    s.kind,
                    SymbolKind::Function
                        | SymbolKind::Method
                        | SymbolKind::ArrowFunction
                        | SymbolKind::Macro
                )
            });
            targets.sort_by(|a, b| a.id.cmp(&b.id));
            targets.dedup_by(|a, b| a.id == b.id);
            for t in &targets {
                if t.id != dc.caller_id {
                    stream_writer.push_calls(
                        &dc.caller_id,
                        &t.id,
                        &dc.site_file,
                        dc.site_start_byte as i64,
                        dc.site_end_byte as i64,
                        true,
                    );
                    calls_emitted += 1;
                }
            }
        }

        // Flush whatever the resolver added.
        stream_writer.flush(store)?;

        info!(
            files = file_known_spurs.len(),
            imports = imports_emitted,
            calls = calls_emitted,
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
    workspace: &Workspace,
    repo_id: &str,
    deferred_imports: &mut Vec<DeferredImport>,
    deferred_calls: &mut Vec<DeferredCall>,
    file_symbols_by_name: &mut HashMap<(Spur, Spur), Vec<AbsorbedSymbol>>,
    file_exports_by_name: &mut HashMap<(Spur, Spur), Vec<AbsorbedSymbol>>,
    file_known_spurs: &mut HashSet<Spur>,
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

    let path_spur = graph.symbols.intern(&path);
    file_known_spurs.insert(path_spur);
    let language_str = language.as_str();

    // *file row + classification + nolints. These used to be emitted by
    // `from_code_graph::emit_node` for `NodeWeight::File`; folding them
    // into absorb lets the File "node" exist only as a Cozo row.
    stream_writer.push_file(&path, language_str, repo_id);
    let src_for_marker = workspace.read_file(&path);
    let is_generated = src_for_marker
        .as_ref()
        .map(|src| is_generated_marker(src))
        .unwrap_or(false);
    stream_writer.push_file_classification(
        &path,
        is_test_file(&path),
        is_barrel_file(&path),
        is_generated,
    );
    if let Some(src) = src_for_marker {
        extract_nolints(&path, &src, stream_writer);
    }

    // Pass 1: compute symbol IDs + populate file-local lookup maps.
    // `local_id_by_line` mirrors the old `graph.symbol_nodes` map
    // semantics — same-line collisions keep whichever symbol the
    // extractor emitted last, so call-site attribution matches the
    // pre-Slice-B baseline.
    let mut symbol_ids: Vec<String> = Vec::with_capacity(symbols.len());
    let mut local_id_by_line: HashMap<u32, String> = HashMap::with_capacity(symbols.len());
    for sym in &symbols {
        let id = symbol_id(
            &sym.file_path,
            sym.start_line,
            sym.start_column,
            &sym.name,
            sym.kind,
        );
        let sym_file_spur = graph.symbols.intern(&sym.file_path);
        let sym_name_spur = graph.symbols.intern(&sym.name);
        let absorbed = AbsorbedSymbol {
            id: id.clone(),
            kind: sym.kind,
        };
        file_symbols_by_name
            .entry((sym_file_spur, sym_name_spur))
            .or_default()
            .push(absorbed.clone());
        if sym.is_exported {
            file_exports_by_name
                .entry((sym_file_spur, sym_name_spur))
                .or_default()
                .push(absorbed);
        }
        graph
            .symbol_ids_by_name
            .entry((sym.file_path.clone(), sym.name.clone()))
            .or_default()
            .push(id.clone());
        local_id_by_line.insert(sym.start_line, id.clone());
        symbol_ids.push(id);
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

    // Compute qualified_name in outer-first order.
    let mut qnames: Vec<String> = vec![String::new(); symbols.len()];
    for &i in &order {
        let sym = &symbols[i];
        qnames[i] = match parent_of[i] {
            Some(p) => format!("{}{}{}", &qnames[p], sep, sym.name),
            None => sym.name.clone(),
        };
    }

    // Stream *symbol + *span rows. parent_id is the parent symbol's
    // stringly id when one exists — pre-Slice-B this was looked up by
    // walking the Contains edge during populate; computing it inline
    // here removes the need for the adjacency lists.
    for (i, sym) in symbols.iter().enumerate() {
        let parent_id = parent_of[i].map(|p| symbol_ids[p].as_str());
        stream_writer.push_symbol(
            &symbol_ids[i],
            sym.kind.to_string().as_str(),
            &sym.name,
            &qnames[i],
            language_str,
            sym.visibility.as_str(),
            &sym.file_path,
            parent_id,
            sym.is_async,
            sym.is_static,
            sym.is_abstract,
            sym.is_mutable,
            sym.is_exported,
        );
        stream_writer.push_span(
            &symbol_ids[i],
            &sym.file_path,
            sym.start_byte as i64,
            sym.end_byte as i64,
            sym.start_line as i64,
            sym.end_line as i64,
            sym.start_column as i64,
            sym.end_column as i64,
        );
    }

    // Stash comments per file so the populate phase can emit `comment`
    // rows. (Comments still need cross-file symbol-id lookup via
    // `graph.symbol_ids_by_name`, so eviction is deferred — Slice A
    // proved it's not the dominant memory term.)
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

    // Defer Calls resolution until every file is absorbed. The original
    // pipeline materialised a `NodeWeight::CallSite` here and walked
    // the adjacency lists later; we now keep just the caller_id + site
    // location, which is everything the post-absorb resolver needs to
    // emit `*calls` rows. CallSite has no Cozo relation of its own —
    // its location folds into the `calls` row's
    // call_site_file/start_byte/end_byte fields.
    for cs in call_sites {
        let Some(caller_id) = local_id_by_line.get(&cs.caller_symbol_line).cloned() else {
            continue;
        };
        let callee_spur = graph.symbols.intern(&cs.callee_name);
        deferred_calls.push(DeferredCall {
            caller_id,
            caller_file_spur: path_spur,
            callee_spur,
            site_file: cs.caller_file,
            site_start_byte: cs.start_byte,
            site_end_byte: cs.end_byte,
        });
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
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cozo::CozoStore;
    use crate::storage::workspace::Workspace;
    use std::collections::BTreeMap;

    /// Build the workspace into a fresh in-memory store. Tests query
    /// the resulting Cozo relations directly — `CodeGraph` no longer
    /// carries the per-symbol/per-edge inspection points the original
    /// tests used.
    fn build_into_store(dir: &std::path::Path, langs: &[Language]) -> CozoStore {
        let ws = Workspace::load(dir, langs, None).unwrap();
        let store = CozoStore::open_in_memory().unwrap();
        GraphBuilder::new(&ws, langs).build(&store).unwrap();
        store
    }

    fn count_calls_total(store: &CozoStore) -> i64 {
        let rows = store
            .run_query("?[count(c)] := *calls{caller_id: c}", BTreeMap::new())
            .unwrap();
        match rows.rows.first().and_then(|r| r.first()) {
            Some(cozo::DataValue::Num(cozo::Num::Int(n))) => *n,
            _ => 0,
        }
    }

    fn calls_from(store: &CozoStore, caller_name: &str) -> Vec<(String, String)> {
        let rows = store
            .run_query(
                "?[callee_name, callee_file] := \
                 *symbol{id: caller_id, name: $name}, \
                 *calls{caller_id, callee_id}, \
                 *symbol{id: callee_id, name: callee_name, file_path: callee_file}",
                BTreeMap::from([("name".to_string(), cozo::DataValue::from(caller_name))]),
            )
            .unwrap();
        rows.rows
            .into_iter()
            .filter_map(|r| {
                let mut it = r.into_iter();
                let callee_name = to_str(&it.next()?)?;
                let callee_file = to_str(&it.next()?)?;
                Some((callee_name, callee_file))
            })
            .collect()
    }

    fn to_str(v: &cozo::DataValue) -> Option<String> {
        match v {
            cozo::DataValue::Str(s) => Some(s.to_string()),
            _ => None,
        }
    }

    #[test]
    fn test_call_resolution_scoped_to_imports_same_name_different_files() {
        // Two files each define `init`; a third file calls `init` and
        // imports only one of them. The Calls row must resolve to the
        // imported `init` and not the unrelated one.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn init() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn init() {}\n").unwrap();
        std::fs::write(
            dir.path().join("c.rs"),
            "use self::a::init;\nfn run() { init(); }\n",
        )
        .unwrap();
        let store = build_into_store(dir.path(), &[Language::Rust]);

        let calls = calls_from(&store, "run");
        assert_eq!(
            calls.len(),
            1,
            "expected exactly 1 Calls row from run, got {:?}",
            calls
        );
        assert_eq!(calls[0].0, "init");
        assert_eq!(calls[0].1, "a.rs");
    }

    #[test]
    fn test_call_resolution_drops_cross_file_edge_without_import() {
        // Caller calls `beta` defined in another file, with no import.
        // Old global-lookup behavior would produce a Calls edge; the
        // import-scoped resolver must produce none.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn alpha() { beta(); }\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn beta() {}\n").unwrap();
        let store = build_into_store(dir.path(), &[Language::Rust]);
        assert_eq!(
            count_calls_total(&store),
            0,
            "expected 0 Calls rows without an import"
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
        let store = build_into_store(dir.path(), &[Language::Rust]);
        assert_eq!(
            count_calls_total(&store),
            0,
            "expected 0 Calls rows when target is not exported"
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
        let store = build_into_store(dir.path(), &[Language::Rust]);
        assert!(
            count_calls_total(&store) >= 1,
            "expected at least 1 same-file Calls row"
        );
    }
}
