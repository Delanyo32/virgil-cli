use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, info_span};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use tree_sitter::Query;

use crate::classify::{is_barrel_file, is_test_file};
use crate::db::from_code_graph::{
    detect_todo_kind, extract_nolints, is_doc_comment, is_generated_marker, symbol_id, type_id,
};
use crate::db::{DbStore, DbWriter};
use crate::graph::GraphNode;
use crate::language::Language;
use crate::languages;
use crate::models::InheritanceKind;
use crate::models::{
    AttrsBucket, CommentInfo, FieldTypeRow, ImportInfo, InheritanceRow, ParameterTypeRow,
    ReferencesBucket, ReturnsTypeRow, SymbolInfo, SymbolKind, ThrowsRow, TypeRow,
};
use crate::parser;
use crate::storage::workspace::Workspace;

use super::{CodeGraph, Spur, Symbols};

/// Flush the streaming writer every this many files. Caps peak writer
/// memory to roughly N files' worth of in-flight rows. Picked to
/// amortise Cozo transaction overhead — too low thrashes SQLite, too
/// high defeats streaming.
const STREAM_FLUSH_EVERY_N_FILES: u32 = 200;

/// Eager import resolution. Build-time resolver maps each
/// `*raw_import{module_specifier}` to a concrete file path using the
/// per-language `languages::resolve_import` logic, then emits
/// `*imports{importer_file_id, imported_id}` rows. Kept eager because
/// per-language module-resolution rules (Node, Python sys.path, Java
/// classpath, etc.) are pages of Rust per language — not practical to
/// express in Cozoscript at query time. Memory cost is small: ~30 MB
/// peak on openclaw (14k files).
const RESOLVE_IMPORTS_EAGERLY: bool = true;

/// Eager call resolution. Previously the build walked every call site,
/// resolved the callee to a symbol id via an import-scoped name lookup
/// (file_symbols_by_name / file_exports_by_name HashMaps), and emitted
/// a `*calls{caller_id, callee_id, ...}` row. On large C++ repos this
/// resolution scratch dominated RAM and caused container OOM at the
/// 4 GiB cap (openclaw, 14k files, peaked at 3.26 GiB → SIGKILL).
///
/// Now disabled: `*calls` rows are no longer materialised at build.
/// Callers compute them at query time via Cozoscript over
/// `*occurrence{occurrence_kind: 'call', enclosing_symbol_id, name}`
/// joined to `*imports` + `*symbol` — same accuracy as the build-time
/// resolver, deferred. See `examples/calls_at_query_time.cozoql` and
/// `docs/resolution.md` for the replacement pattern; the affected
/// built-in templates (`find_callers`, `find_callees`, `find_cycles`)
/// have been rewritten to use it.
const RESOLVE_CALLS_EAGERLY: bool = false;

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
    receiver: Option<String>,
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

/// Shared scratch state for the parallel parse+absorb pass. All rayon
/// workers contend on one `Mutex<SharedAbsorb>` while absorbing — the
/// critical section is short (Vec appends + HashMap inserts) compared
/// to per-file parsing, so contention isn't the bottleneck. Replaces
/// the prior per-worker `WorkerLocal` design, which held ~850 MiB of
/// scratch state at peak on a 5k-file TS corpus.
struct SharedAbsorb {
    writer: DbWriter,
    deferred_imports: Vec<DeferredImport>,
    deferred_calls: Vec<DeferredCall>,
    file_symbols_by_name: HashMap<(Spur, Spur), Vec<AbsorbedSymbol>>,
    file_exports_by_name: HashMap<(Spur, Spur), Vec<AbsorbedSymbol>>,
    file_known_spurs: HashSet<Spur>,
    /// Files absorbed since the last flush. Triggers a `writer.flush`
    /// every `STREAM_FLUSH_EVERY_N_FILES` to cap peak memory.
    files_since_flush: u32,
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

    pub fn build(&self, store: &DbStore) -> Result<CodeGraph> {
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

        // Step 4 + 5: Parallel parse, shared-writer absorption.
        //
        // Rayon workers each parse a file (CPU-heavy, no shared state),
        // then briefly lock a shared `SharedAbsorb` to push the
        // extracted rows into a single `DbWriter` + the cross-file
        // deferred Vecs + the interner. This was previously done via
        // a per-worker `WorkerLocal` reduced at the end, which held
        // ~850 MiB of scratch state at peak (measured 2026-05-27).
        // Sharing the writer keeps memory near baseline; the absorb
        // critical section is short (just appends to Vecs) so the
        // mutex doesn't dominate wall time.
        let pool = rayon::ThreadPoolBuilder::new()
            .stack_size(4 * 1024 * 1024)
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

        // Shared interner. Cloning is cheap (just bumps an Arc).
        let shared_symbols = Symbols::new();
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

        let (
            mut stream_writer,
            deferred_imports,
            deferred_calls,
            file_symbols_by_name,
            file_exports_by_name,
            file_known_spurs,
        ) = {
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

            let parsed_ref = &parsed;
            let absorbed_ref = &absorbed_files;
            let repo_id_ref = repo_id.as_str();
            let interner = &shared_symbols;

            // One shared writer + cross-file scratch, behind a mutex.
            // The lock is held only across `absorb_file_data` (Vec
            // appends + a few HashMap inserts) plus the periodic
            // flush. Parsing happens lock-free.
            let shared = Mutex::new(SharedAbsorb {
                writer: DbWriter::new(),
                deferred_imports: Vec::new(),
                deferred_calls: Vec::new(),
                file_symbols_by_name: HashMap::new(),
                file_exports_by_name: HashMap::new(),
                file_known_spurs: HashSet::new(),
                files_since_flush: 0,
            });

            pool.install(|| -> Result<()> {
                grouped_files_ref
                    .par_iter()
                    .try_for_each(|&(lang, rel_path)| -> Result<()> {
                        let Some(data) =
                            parse_one_file(lang, rel_path, workspace, &sym_q, &imp_q, &com_q)
                        else {
                            return Ok(());
                        };
                        parsed_ref.fetch_add(1, Ordering::Relaxed);
                        let mut state = shared.lock().expect("shared absorb mutex poisoned");
                        let state = &mut *state;
                        absorb_file_data(
                            interner,
                            data,
                            workspace,
                            repo_id_ref,
                            &mut state.deferred_imports,
                            &mut state.deferred_calls,
                            &mut state.file_symbols_by_name,
                            &mut state.file_exports_by_name,
                            &mut state.file_known_spurs,
                            &mut state.writer,
                        );
                        absorbed_ref.fetch_add(1, Ordering::Relaxed);
                        tracing::Span::current().pb_inc(1);
                        state.files_since_flush += 1;
                        if state.files_since_flush >= STREAM_FLUSH_EVERY_N_FILES {
                            state.writer.flush(store)?;
                            state.files_since_flush = 0;
                        }
                        Ok(())
                    })
            })?;

            let SharedAbsorb {
                writer: mut stream_writer,
                deferred_imports,
                deferred_calls,
                file_symbols_by_name,
                file_exports_by_name,
                file_known_spurs,
                ..
            } = shared.into_inner().expect("shared absorb mutex poisoned");
            // Flush the writer's tail rows before cross-file resolution
            // runs — keeps populate's later phases from racing with
            // leftover per-file rows.
            stream_writer.flush(store)?;
            info!(
                parsed = parsed.load(Ordering::Relaxed),
                absorbed = absorbed_files.load(Ordering::Relaxed),
                deferred_imports = deferred_imports.len(),
                deferred_calls = deferred_calls.len(),
                "parse + absorb done"
            );
            (
                stream_writer,
                deferred_imports,
                deferred_calls,
                file_symbols_by_name,
                file_exports_by_name,
                file_known_spurs,
            )
        };
        // CodeGraph is a vestigial wrapper around the shared interner
        // after the SQL-staging refactor; still returned to keep the
        // public API stable for callers that take a `&CodeGraph`.
        let graph = CodeGraph::with_symbols(shared_symbols);

        let _resolve_span = info_span!("graph.resolve_refs").entered();

        // Resolve deferred imports now that every file has been
        // absorbed. Emit `*imports` Cozo rows directly; the in-memory
        // `file_imports` map only exists long enough for the call
        // resolver to scope its lookups (when eager calls are enabled).
        // C# `using X` imports a namespace, not a file path. Build a
        // namespace -> declaring-files index from the absorbed `symbol` rows
        // so those imports resolve to every file that declares the namespace.
        let namespace_files = if self.languages.contains(&Language::CSharp) {
            build_namespace_index(store)?
        } else {
            HashMap::new()
        };
        let mut file_imports: HashMap<Spur, Vec<Spur>> = HashMap::new();
        let mut imports_emitted: usize = 0;
        for di in deferred_imports {
            let Some(from_spur) = graph.symbols.get(&di.from_file_path) else {
                continue;
            };
            if !file_known_spurs.contains(&from_spur) {
                continue;
            }
            // A `File` resolves to one workspace file; a `Package` (Go) resolves
            // to a directory — every file under it is a dependency, so fan out.
            // C# resolves a `using` namespace to every file declaring it.
            let targets: Vec<String> = if di.language == Language::CSharp {
                namespace_files
                    .get(&di.import.module_specifier)
                    .cloned()
                    .unwrap_or_default()
            } else {
                match resolve_import_to_node(
                    &di.from_file_path,
                    &di.import,
                    di.language,
                    &known_files,
                ) {
                    Some(GraphNode::File(p)) => vec![p],
                    Some(GraphNode::Package(dir)) => {
                        let prefix = format!("{dir}/");
                        known_files
                            .iter()
                            .filter(|f| f.starts_with(&prefix))
                            .cloned()
                            .collect()
                    }
                    None => continue,
                }
            };
            for resolved in targets {
                if let Some(to_spur) = graph.symbols.get(&resolved)
                    && file_known_spurs.contains(&to_spur)
                    && from_spur != to_spur
                {
                    stream_writer.push_imports(&di.from_file_path, &resolved);
                    file_imports.entry(from_spur).or_default().push(to_spur);
                    imports_emitted += 1;
                }
            }
        }

        // Resolve deferred Calls with import-scoped name lookup:
        //
        //   - Same-file: any symbol in the caller's file with a matching name.
        //   - Cross-file: only symbols whose file the caller imports, and
        //     which are themselves exported.
        //
        // Disabled by default (RESOLVE_CALLS_EAGERLY = false); call
        // edges are derived at query time over `*occurrence` +
        // `*imports` + `*symbol`. See `examples/calls_at_query_time.cozoql`
        // and `docs/resolution.md`. The block is kept compiled (rather
        // than deleted) so the eager path can be re-enabled without a
        // refactor if some future workload makes the RAM/latency
        // trade-off worth flipping again.
        let mut calls_emitted: usize = 0;
        if RESOLVE_CALLS_EAGERLY {
            let mut targets: Vec<AbsorbedSymbol> = Vec::new();
            for dc in deferred_calls {
                targets.clear();

                if let Some(syms) = file_symbols_by_name.get(&(dc.caller_file_spur, dc.callee_spur))
                {
                    targets.extend(syms.iter().cloned());
                }

                if let Some(imp_files) = file_imports.get(&dc.caller_file_spur) {
                    for &imp_file in imp_files {
                        if let Some(syms) = file_exports_by_name.get(&(imp_file, dc.callee_spur)) {
                            targets.extend(syms.iter().cloned());
                        }
                    }
                }

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
        } else {
            // Consume the bindings so the compiler doesn't warn when
            // the eager block is gated off.
            let _ = (
                deferred_calls,
                file_symbols_by_name,
                file_exports_by_name,
                file_imports,
            );
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
    // Function-like symbols only — parameters and locals can't enclose
    // calls. Pre-collect their line ranges so a single tree walk can
    // attribute each call to its innermost enclosing function.
    //
    // Prior shape was a loop over every function symbol that re-walked
    // the whole tree scoped to the symbol's range; nested functions
    // therefore emitted the same call_site twice (once per enclosing
    // symbol), producing duplicate ids that Cozo silently upserted
    // and DuckDB's strict Appender rejects with a PK violation.
    let caller_ranges: Vec<(u32, u32, u32)> = symbols
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Function
                    | SymbolKind::Method
                    | SymbolKind::ArrowFunction
                    | SymbolKind::Macro
            )
        })
        .map(|s| (s.start_line, s.end_line, s.start_line))
        .collect();
    collect_calls(
        tree.root_node(),
        source.as_bytes(),
        &call_node_types,
        lang,
        rel_path,
        &caller_ranges,
        &mut call_sites,
    );

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
    interner: &Symbols,
    data: FileGraphData,
    workspace: &Workspace,
    repo_id: &str,
    deferred_imports: &mut Vec<DeferredImport>,
    deferred_calls: &mut Vec<DeferredCall>,
    file_symbols_by_name: &mut HashMap<(Spur, Spur), Vec<AbsorbedSymbol>>,
    file_exports_by_name: &mut HashMap<(Spur, Spur), Vec<AbsorbedSymbol>>,
    file_known_spurs: &mut HashSet<Spur>,
    stream_writer: &mut DbWriter,
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

    let path_spur = interner.intern(&path);
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
        let sym_file_spur = interner.intern(&sym.file_path);
        let sym_name_spur = interner.intern(&sym.name);
        // file_symbols_by_name / file_exports_by_name only feed the
        // eager calls resolver. When that's disabled, skip them — they
        // were the dominant RAM term on large C++ repos.
        if RESOLVE_CALLS_EAGERLY {
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
        } else {
            let _ = (sym_file_spur, sym_name_spur);
        }
        // Only function-like symbols can enclose a call, and call-site
        // attribution (`collect_calls`) keys the caller on a function-like
        // symbol's start_line. Parameters/locals/fields share that line
        // (`function foo(a, b)` — `a`,`b` start on the signature line) and
        // would clobber the slot, mis-attributing the caller to a parameter.
        // Mirror the `caller_ranges` filter so the lookup is consistent.
        if matches!(
            sym.kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::ArrowFunction
                | SymbolKind::Macro
        ) {
            local_id_by_line.insert(sym.start_line, id.clone());
        }
        symbol_ids.push(id);
    }

    // File-local symbol lookup. Built up front so the populate-tail
    // emitters below can resolve `(file, name)` -> symbol_id without
    // round-tripping to DuckDB or stashing on a global CodeGraph map.
    // Same-file collisions: keep the first (mirrors the prior
    // `pick_symbol_id` behaviour of `v.first()`).
    let mut name_to_id: HashMap<&str, &str> = HashMap::with_capacity(symbols.len());
    for (i, sym) in symbols.iter().enumerate() {
        name_to_id
            .entry(sym.name.as_str())
            .or_insert(&symbol_ids[i]);
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

    // Emit comment rows file-locally. `documents_id` is resolved
    // against the same file's name_to_id map. Pre-refactor this lived
    // on `graph.comments` and was emitted by `emit_comments` in the
    // populate phase against `graph.symbol_ids_by_name`.
    for (i, c) in comments.iter().enumerate() {
        let id = format!("{}|{}|{}|comment", path, c.start_byte, i);
        let documents_id = c
            .associated_symbol
            .as_ref()
            .and_then(|name| name_to_id.get(name.as_str()).copied());
        let is_doc = is_doc_comment(&c.kind, &c.text);
        let todo_kind = detect_todo_kind(&c.text);
        stream_writer.push_comment(
            &id,
            documents_id,
            &path,
            &c.kind,
            is_doc,
            &c.text,
            todo_kind,
            c.start_byte as i64,
            c.end_byte as i64,
        );
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
    if RESOLVE_IMPORTS_EAGERLY {
        for import in imports {
            deferred_imports.push(DeferredImport {
                from_file_path: path.clone(),
                language,
                import,
            });
        }
    } else {
        let _ = imports;
    }

    // Emit per-file type / signature / field / throws rows directly.
    // The populate-phase Rust resolver these used to feed has been
    // deleted — name resolution that used to look up
    // `graph.symbol_ids_by_name` is now either purely file-local
    // (uses `name_to_id` above) or staged for SQL (inheritance).
    let mut type_id_by_display: HashMap<&str, String> = HashMap::with_capacity(types.len());
    for row in &types {
        let id = type_id(language_str, &path, &row.display_name);
        type_id_by_display
            .entry(row.display_name.as_str())
            .or_insert_with(|| id.clone());
        stream_writer.push_type(
            &id,
            &row.kind,
            language_str,
            &row.display_name,
            row.canonical_name.as_deref(),
        );
    }
    for row in &param_types {
        let function_id = symbol_id(
            &path,
            row.function_start_line,
            row.function_start_col,
            &row.function_name,
            row.function_kind,
        );
        let param_id = symbol_id(
            &path,
            row.parameter_start_line,
            row.parameter_start_col,
            &row.parameter_name,
            SymbolKind::Parameter,
        );
        let type_id_str = row.type_display_name.as_ref().map(|d| {
            type_id_by_display
                .get(d.as_str())
                .cloned()
                .unwrap_or_else(|| type_id(language_str, &path, d))
        });
        stream_writer.push_parameter(
            &param_id,
            &row.parameter_name,
            &function_id,
            row.position,
            type_id_str.as_deref(),
            row.is_optional,
            row.has_default,
            false,
        );
    }
    for row in &returns_types {
        let function_id = symbol_id(
            &path,
            row.function_start_line,
            row.function_start_col,
            &row.function_name,
            row.function_kind,
        );
        let tid = type_id_by_display
            .get(row.type_display_name.as_str())
            .cloned()
            .unwrap_or_else(|| type_id(language_str, &path, &row.type_display_name));
        stream_writer.push_returns_type(&function_id, &tid);
    }
    for row in &throws {
        let function_id = symbol_id(
            &path,
            row.function_start_line,
            row.function_start_col,
            &row.function_name,
            row.function_kind,
        );
        // Throws can reference exception names that the type extractor
        // never emitted — synthesise a `named` type row to keep the
        // 3-way JOIN through `type` non-empty (matches the prior
        // `emit_types_and_hierarchy` behaviour).
        let tid =
            if let Some(existing) = type_id_by_display.get(row.exception_display_name.as_str()) {
                existing.clone()
            } else {
                let id = type_id(language_str, &path, &row.exception_display_name);
                type_id_by_display.insert(row.exception_display_name.as_str(), id.clone());
                stream_writer.push_type(
                    &id,
                    "named",
                    language_str,
                    &row.exception_display_name,
                    None,
                );
                id
            };
        stream_writer.push_throws(&function_id, &tid);
    }
    for row in &field_types {
        let field_symbol_id = symbol_id(
            &path,
            row.field_start_line,
            row.field_start_col,
            &row.field_name,
            row.field_kind,
        );
        let tid = type_id_by_display
            .get(row.type_display_name.as_str())
            .cloned()
            .unwrap_or_else(|| type_id(language_str, &path, &row.type_display_name));
        stream_writer.push_field_type(&field_symbol_id, &tid);
    }
    // Inheritance is the only extractor output that needs cross-file
    // symbol-id resolution (parent class may live in another file).
    // Stage as a raw row in DuckDB; a SQL resolver in
    // `db::from_code_graph::resolve_inheritance` runs after parse.
    for row in &inheritance {
        let parent_leaf = row
            .parent_display_name
            .rsplit("::")
            .next()
            .unwrap_or(&row.parent_display_name)
            .trim_end_matches('>')
            .split('<')
            .next()
            .unwrap_or("")
            .trim();
        let kind_str = match row.kind {
            InheritanceKind::Extends => "extends",
            InheritanceKind::Implements => "implements",
        };
        stream_writer.push_raw_inheritance(
            &path,
            &row.child_name,
            row.child_kind.to_string().as_str(),
            row.child_start_line as i64,
            row.child_start_col as i64,
            parent_leaf,
            row.parent_canonical_name.as_deref(),
            kind_str,
        );
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

    // Emit one `*call_site` row per call expression. caller_id is
    // None for calls that sit outside any function/method (e.g.
    // top-level script statements). `id` is a deterministic
    // `file:start_byte` slug — each call site has at most one row.
    //
    // When eager resolution is enabled (RESOLVE_CALLS_EAGERLY), we
    // ALSO push a DeferredCall so the post-channel resolver can emit
    // *calls rows. Templates that want resolved call edges read
    // *call_site at query time — see find_callers.cozoql.
    for cs in call_sites {
        let caller_id_opt = local_id_by_line.get(&cs.caller_symbol_line).cloned();
        // Nested calls (`super().__init__(...)` etc.) share start_byte
        // — inner and outer call expressions start at the same source
        // position. Include `end_byte` in the key so the outer call
        // (longer range) doesn't get overwritten by the inner one.
        let site_id = format!("{}:{}:{}", cs.caller_file, cs.start_byte, cs.end_byte);
        stream_writer.push_call_site(
            &site_id,
            caller_id_opt.as_deref(),
            &cs.callee_name,
            cs.receiver.as_deref(),
            &cs.caller_file,
            cs.start_byte as i64,
            cs.end_byte as i64,
        );

        if RESOLVE_CALLS_EAGERLY {
            let Some(caller_id) = caller_id_opt else {
                continue;
            };
            let callee_spur = interner.intern(&cs.callee_name);
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
    if !RESOLVE_CALLS_EAGERLY {
        let _ = path_spur;
    }
}

/// Resolve an import to its graph node (`File` for path-granular languages,
/// `Package` for Go's directory-granular imports).
fn resolve_import_to_node(
    source_file: &str,
    import: &ImportInfo,
    language: Language,
    known_files: &HashSet<String>,
) -> Option<GraphNode> {
    languages::resolve_import(source_file, import, language, known_files)
}

/// Build a `namespace name -> declaring files` index from the absorbed
/// `symbol` rows. Used to resolve C# `using` imports, which reference a
/// namespace rather than a file path; a namespace may be declared across
/// several files. Must run after the writer is flushed.
fn build_namespace_index(store: &DbStore) -> Result<HashMap<String, Vec<String>>> {
    store.with_conn(|conn| {
        let mut index: HashMap<String, Vec<String>> = HashMap::new();
        let mut stmt =
            conn.prepare("SELECT name, file_path FROM symbol WHERE kind = 'namespace'")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let name: String = r.get(0)?;
            let file_path: String = r.get(1)?;
            index.entry(name).or_default().push(file_path);
        }
        Ok(index)
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
        // tree-sitter-php names member calls `member_call_expression`
        // (`$o->m()`) / `scoped_call_expression` (`C::m()`) /
        // `nullsafe_member_call_expression` (`$o?->m()`) — there is no
        // `method_call_expression` node, so those calls were previously
        // never collected.
        Language::Php => vec![
            "function_call_expression",
            "member_call_expression",
            "scoped_call_expression",
            "nullsafe_member_call_expression",
        ],
    }
}

/// Single-pass tree walk. For each call expression, picks the
/// innermost enclosing function-like symbol (smallest line range
/// containing the call) as the caller. `caller_ranges` is a
/// `(start_line, end_line, caller_symbol_line)` triple per
/// function-like symbol; `caller_symbol_line` is the value the
/// downstream resolver looks up in `local_id_by_line`. Top-level
/// calls (not inside any function-like symbol) get
/// `caller_symbol_line = 0`, which downstream maps to `caller_id =
/// NULL`.
fn collect_calls(
    node: tree_sitter::Node,
    source: &[u8],
    call_types: &[&str],
    language: Language,
    file_path: &str,
    caller_ranges: &[(u32, u32, u32)],
    out: &mut Vec<CallSiteData>,
) {
    if call_types.contains(&node.kind())
        && let Some((name, receiver)) = extract_callee_name(node, source, language)
    {
        let node_line = node.start_position().row as u32 + 1;
        // Innermost = smallest end_line - start_line span that contains node_line.
        let caller_symbol_line = caller_ranges
            .iter()
            .filter(|(s, e, _)| *s <= node_line && *e >= node_line)
            .min_by_key(|(s, e, _)| e.saturating_sub(*s))
            .map(|(_, _, l)| *l)
            .unwrap_or(0);
        out.push(CallSiteData {
            callee_name: name,
            receiver,
            caller_file: file_path.to_string(),
            caller_symbol_line,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(
            child,
            source,
            call_types,
            language,
            file_path,
            caller_ranges,
            out,
        );
    }
}

/// Returns `(callee_name, receiver)` for a call node.
///
/// `callee_name` is the bare method/function name (text-split on the
/// trailing `.` / `::`), kept identical across the swap so call
/// resolution is unaffected.
///
/// `receiver` is the object/namespace the call is made on, read directly
/// from the grammar's named fields rather than by splitting text — this
/// is accurate across every language's call syntax (`.`, `->`, `::`,
/// chains, computed access). `None` for bare calls like `foo()`. See
/// [`call_receiver`] for the per-grammar field map.
fn extract_callee_name(
    node: tree_sitter::Node,
    source: &[u8],
    _language: Language,
) -> Option<(String, Option<String>)> {
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
        return None;
    }

    let receiver = call_receiver(node, func_node)
        .and_then(|r| r.utf8_text(source).ok())
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string);

    Some((name.to_string(), receiver))
}

/// Locate the receiver node of a call by its grammar field, covering all
/// supported languages. Two shapes:
///
/// - **call-node fields** — the call node itself carries the receiver
///   (Java `method_invocation`, PHP `member_call_expression` →
///   `object`; PHP `scoped_call_expression` → `scope`).
/// - **callee-node fields** — the callee is a member-access node whose
///   receiver is a field on it: JS/TS `member_expression` & Python
///   `attribute` → `object`; Go `selector_expression` → `operand`;
///   C# `member_access_expression` → `expression`; Rust `field_expression`
///   → `value` / `scoped_identifier` → `path`; C/C++ `field_expression`
///   → `argument`.
///
/// Field names verified against the bundled tree-sitter grammars. A bare
/// call (`foo()`) matches none of these and yields `None`.
fn call_receiver<'t>(
    call: tree_sitter::Node<'t>,
    callee: tree_sitter::Node<'t>,
) -> Option<tree_sitter::Node<'t>> {
    call.child_by_field_name("object")
        .or_else(|| call.child_by_field_name("scope"))
        .or_else(|| callee.child_by_field_name("object"))
        .or_else(|| callee.child_by_field_name("operand"))
        .or_else(|| callee.child_by_field_name("expression"))
        .or_else(|| callee.child_by_field_name("value"))
        .or_else(|| callee.child_by_field_name("path"))
        .or_else(|| callee.child_by_field_name("argument"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbStore;
    use crate::db::from_code_graph as fcg;
    use crate::storage::workspace::Workspace;
    use duckdb::types::Value;
    use std::collections::BTreeMap;

    /// Build the workspace into a fresh in-memory store and run the
    /// populate tail so `*call_edge` is materialised. Tests query the
    /// resulting DuckDB tables directly.
    fn build_into_store(dir: &std::path::Path, langs: &[Language]) -> DbStore {
        let ws = Workspace::load(dir, langs, None).unwrap();
        let store = DbStore::open_in_memory().unwrap();
        let graph = GraphBuilder::new(&ws, langs).build(&store).unwrap();
        fcg::populate(&store, &graph, Some(&ws)).unwrap();
        store
    }

    fn count_calls_total(store: &DbStore) -> i64 {
        let rows = store
            .run_query("SELECT COUNT(*) FROM call_edge", BTreeMap::new())
            .unwrap();
        match rows.rows.first().and_then(|r| r.first()) {
            Some(Value::BigInt(n)) => *n,
            Some(Value::Int(n)) => *n as i64,
            _ => 0,
        }
    }

    fn calls_from(store: &DbStore, caller_name: &str) -> Vec<(String, String)> {
        let sql = format!(
            "SELECT callee.name, callee.file_path \
             FROM call_edge ce \
             JOIN symbol caller ON caller.id = ce.caller_id \
             JOIN symbol callee ON callee.id = ce.callee_id \
             WHERE caller.name = '{}'",
            caller_name.replace('\'', "''")
        );
        let rows = store.run_query(&sql, BTreeMap::new()).unwrap();
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

    fn to_str(v: &Value) -> Option<String> {
        match v {
            Value::Text(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Resolved import targets (`imported_id`) for a given importer file.
    fn imports_targets(store: &DbStore, importer: &str) -> Vec<String> {
        let sql = format!(
            "SELECT imported_id FROM imports WHERE importer_file_id = '{}' ORDER BY imported_id",
            importer.replace('\'', "''")
        );
        let rows = store.run_query(&sql, BTreeMap::new()).unwrap();
        rows.rows
            .into_iter()
            .filter_map(|r| to_str(r.first()?))
            .collect()
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

    #[test]
    fn csharp_using_resolves_to_namespace_declaring_file() {
        // `using App.Models` resolves to the file declaring that namespace;
        // `using System` (no workspace declarer) must not resolve.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Models.cs"),
            "namespace App.Models { public class User {} }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Services.cs"),
            "using System;\nusing App.Models;\nnamespace App.Services { public class UserService {} }\n",
        )
        .unwrap();
        let store = build_into_store(dir.path(), &[Language::CSharp]);
        assert_eq!(
            imports_targets(&store, "Services.cs"),
            vec!["Models.cs".to_string()]
        );
    }

    #[test]
    fn go_package_import_fans_out_to_package_files() {
        // A Go package import resolves to a directory; every file in that
        // package becomes an imports edge (exercises the gate bypass + fan-out).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("internal/util")).unwrap();
        std::fs::write(
            dir.path().join("internal/util/helper.go"),
            "package util\nfunc Help() {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.go"),
            "package main\nimport \"example.com/app/internal/util\"\nfunc main() {}\n",
        )
        .unwrap();
        let store = build_into_store(dir.path(), &[Language::Go]);
        assert_eq!(
            imports_targets(&store, "main.go"),
            vec!["internal/util/helper.go".to_string()]
        );
    }
}
