# Plan: replace virgil-cli's pipeline DSL with Cozoscript

## Goal

**Cozoscript (Datalog) becomes virgil-cli's query language.** The legacy
JSON pipeline DSL, the pipeline executor, all pipeline stages, all
built-in audit pipelines, and the `audit` subcommand are **deleted, not
ported**. The `projects query` subcommand becomes the primary surface,
accepting Cozoscript directly.

End state:
- `cargo tree | grep petgraph` returns nothing.
- `src/pipeline/`, `src/audit/`, `src/graph/taint/`, `src/graph/resource.rs`
  do not exist.
- Cross-function graph + per-function CFG facts + precomputed metrics all
  live as CozoDB relations.
- Users query the graph with `projects query <name> --file q.cozoql` (or
  inline `--cozoscript '...'`, or `--template <name> --param k=v`).
- Net Rust LOC after migration: **~22,000 lines deleted, ~3,000 added.**

**Non-goals**
- Porting the JSON DSL to Cozoscript. Old DSL just gets deleted.
- Migrating the 297 built-in audit JSON files. They're deleted.
- Backward compatibility with the `audit` subcommand. It's deleted.
- Tree-sitter / parser changes.
- Server/daemon — CLI stays embedded.

## What we keep

| Subsystem | Reason |
|---|---|
| Tree-sitter parsing + 10 language modules | Source of all facts; nothing replaces this |
| Language CFG builders | Rewritten to emit Cozo facts instead of building `DiGraph<BasicBlock, CfgEdge>` |
| Tree-sitter-driven metrics (`compute_cyclomatic`, `compute_cognitive`, `compute_nesting_depth`, `compute_function_length`, `compute_comment_ratio`) | Run once at build time, emit values as Cozo facts |
| `Workspace`, `FileSource`, S3 backend, `DiskFileSource` LRU | Disk I/O layer, unchanged |
| `projects` registry CLI (`create`, `list`, `delete`) | Unchanged |
| `projects query --read <file>` for raw file content | Unchanged |
| `Symbols` interner (lasso) | Used internally during build; values resolved to strings before writing Cozo |
| `serve` mode | Unchanged surface; backed by Cozo now |

## What we delete

| Path | Approx LOC | Notes |
|---|---|---|
| `src/pipeline/dsl/` | ~1,400 | DSL types, where_clause, stages enum |
| `src/pipeline/stages/` | ~1,100 | select, aggregate, cycles, match_pattern, find_duplicates, taint, compute_metric |
| `src/pipeline/executor.rs` | ~1,570 | Stage dispatcher, severity resolution, message interpolation |
| `src/pipeline/helpers.rs` | ~150 | is_test_file/is_barrel_file/is_excluded — moved to build-time facts |
| `src/pipeline/loader.rs` | ~330 | JSON pipeline file discovery |
| `src/pipeline/node_helpers.rs` | ~170 | PipelineNode construction helpers |
| `src/pipeline/output.rs` | ~70 | AuditFinding/QueryResult shapes — replaced by Cozo row schema |
| `src/audit/` (entire dir) | ~600 + ~15k JSON | engine.rs, models.rs, format.rs, 297 audit JSON files in builtin/ |
| `src/graph/taint/` (entire dir) | ~1,500 | Replaced by Datalog rules |
| `src/graph/resource.rs` | ~700 | Replaced by Datalog rules |
| `src/graph/cfg.rs` | ~100 | `FunctionCfg`/`BasicBlock`/etc. — replaced by Cozo relations |
| `src/graph/mod.rs` CFG bits | ~80 | `cfg_for_function`, `cfg_cache`, `inject_cfg`, `function_cfg_indices`, `ensure_resource_graph` |
| `src/main.rs` `audit` subcommand handler | ~200 | gone |
| `petgraph` dependency | n/a | dropped from `Cargo.toml` |

**Total deletion: ~22,000+ lines (Rust + JSON).**

## What we add

| Path | Approx LOC | Purpose |
|---|---|---|
| `src/cozo/schema.rs` | ~300 | All `:create` and `::index` statements, schema version |
| `src/cozo/store.rs` | ~250 | `CozoStore` wrapper around `DbInstance`; lifecycle, transactions |
| `src/cozo/writer.rs` | ~400 | Batched row writer fed by parse workers via channel |
| `src/cozo/queries.rs` | ~150 | Typed Cozoscript query constructors (parameter binding, no string concatenation) |
| `src/graph/cfg/facts.rs` | ~250 | `CfgFactBuilder` — write-only accumulator the language CFG builders emit into |
| `src/graph/cfg/emit.rs` | ~100 | Drain `CfgFactBuilder` into Cozo rows |
| `src/cli/query.rs` | ~250 | `--cozoscript`, `--file`, `--template`, `--param` flag handlers |
| `src/queries/builtin/*.cozoql` | ~17 files | Phase 5 ships 10: `find_callers`, `find_callees`, `find_cycles`, `find_function_by_name`, `complexity_hotspots`, `export_surface`, `taint_paths`, `unreleased_resources`, `import_depth`, `unused_symbols`. Phase 10 adds 7 more using Cozo `graph-algo`: `shortest_call_chain`, `k_alternative_paths`, `function_pagerank`, `bridge_functions`, `module_clusters`, `import_pagerank`, `unreachable_from_main`. |
| `Cargo.toml` cozo dep | n/a | Add cozo with rocksdb + sqlite + graph-algo features |

**Total addition: ~2,000–3,000 Rust lines + a handful of `.cozoql` files.**

## Crate landscape

```toml
[dependencies]
cozo = { version = "0.7", default-features = false, features = [
    "storage-rocksdb",      # production path
    "storage-sqlite",       # pure-Rust fallback (bundled sqlite via sqlite3-src)
    "graph-algo",           # ConnectedComponents, ShortestPath, PageRank
] }
lasso = { version = "0.7", features = ["multi-threaded"] }   # keep

# Removed:
# petgraph = "..."   <- gone after this migration
```

References
- Cozo docs: <https://docs.cozodb.org/>
- Cozoscript Datalog: <https://docs.cozodb.org/en/latest/manual/datalog.html>
- Cozo storage backends: <https://docs.cozodb.org/en/latest/manual/storage.html>
- Cozo graph algos: <https://docs.cozodb.org/en/latest/manual/algorithms.html>

## New CLI surface

```bash
# Inline Cozoscript
virgil-cli projects query openclaw \
    --cozoscript '?[name, file] := *symbol{name, file_path: file, exported: true}'

# Cozoscript from file
virgil-cli projects query openclaw --file audits/excessive_api.cozoql

# Pre-canned template with parameter binding (safe, no injection)
virgil-cli projects query openclaw \
    --template find_callers \
    --param target=login \
    --param depth=3

# Raw file content (unchanged)
virgil-cli projects query openclaw --read src/main.rs --lines 10-50

# Output formats (unchanged)
virgil-cli projects query openclaw --file q.cozoql --format json|csv|table

# Convention for audit-shaped output:
# A query returning columns (file, line, severity, pattern, message)
# is auto-formatted as audit findings.
```

**Removed:**
```bash
virgil-cli audit ...                              # subcommand gone
virgil-cli projects query --q '{"find": ...}'     # JSON DSL gone
```

## CozoDB schema

### Cross-function graph

```cozoscript
:create file       {path: String => language: String}
:create symbol     {id: Int => name: String, kind: String, file_path: String,
                    start_line: Int, end_line: Int, exported: Bool}
:create callsite   {id: Int => name: String, file_path: String, line: Int,
                    caller_symbol_id: Int?, enclosing_test_name: String?}
:create call_arg   {callsite_id: Int, position: Int => value: String}
:create parameter  {id: Int => name: String, function_id: Int, position: Int,
                    is_taint_source: Bool}
:create external_source {id: Int => kind: String, file_path: String, line: Int}

:create edge_defined_in   {symbol_id: Int, file_path: String}
:create edge_calls        {caller_id: Int, callee_id: Int}
:create edge_imports      {from_path: String, to_path: String}
:create edge_exports      {file_path: String, symbol_id: Int}
:create edge_contains     {parent_id: Int, child_id: Int}
```

### Per-function CFG facts

```cozoscript
:create cfg_block             {function_id: Int, block_id: Int =>
                               is_entry: Bool, is_exit: Bool}
:create cfg_edge              {function_id: Int, from_block: Int, to_block: Int =>
                               kind: String}
:create cfg_assign            {function_id: Int, block_id: Int, statement_idx: Int =>
                               line: Int, target: String, source_vars: [String]}
:create cfg_call              {function_id: Int, block_id: Int, statement_idx: Int =>
                               line: Int, callee_name: String, args: [String]}
:create cfg_return            {function_id: Int, block_id: Int, statement_idx: Int =>
                               line: Int, value_vars: [String]}
:create cfg_guard             {function_id: Int, block_id: Int, statement_idx: Int =>
                               line: Int, condition_vars: [String]}
:create cfg_resource_acquire  {function_id: Int, block_id: Int, statement_idx: Int =>
                               line: Int, target: String, resource_type: String}
:create cfg_resource_release  {function_id: Int, block_id: Int, statement_idx: Int =>
                               line: Int, target: String, resource_type: String}
:create cfg_phi               {function_id: Int, block_id: Int, statement_idx: Int =>
                               target: String, sources: [String]}
```

### Precomputed metrics (run at build time, stored as facts)

```cozoscript
:create metric_cyclomatic_complexity {symbol_id: Int => value: Int}
:create metric_cognitive_complexity  {symbol_id: Int => value: Int}
:create metric_function_length       {symbol_id: Int => value: Int}
:create metric_nesting_depth         {symbol_id: Int => value: Int}
:create metric_comment_to_code_ratio {file_path: String => value: Int}
:create metric_afferent_coupling     {symbol_id: Int => value: Int}
:create metric_efferent_coupling     {symbol_id: Int => value: Int}
```

These are computed during build (tree-sitter analysis per function) and
stored as facts. Queries join against them instead of recomputing.

### Derived facts

```cozoscript
:create file_classification {path: String =>
                             is_test: Bool, is_barrel: Bool, is_generated: Bool}
:create nolint              {file_path: String, line: Int =>
                             suppressed_pattern: String}
:create taint_source_pattern    {name: String => description: String, kind: String}
:create taint_sanitizer_pattern {name: String => description: String}
:create taint_sink_pattern      {name: String => vulnerability: String}
```

`file_classification` computed once per file at build. `nolint` extracted
from comments during parse. Taint patterns are bound at query time via
parameter relations.

### Indices

```cozoscript
::index create symbol:by_name         {name}
::index create symbol:by_file_line    {file_path, start_line}
::index create callsite:by_name       {name}
::index create callsite:by_file       {file_path}
::index create edge_calls:by_callee   {callee_id}
::index create edge_imports:by_to     {to_path}
::index create cfg_block:by_fn        {function_id}
::index create cfg_edge:by_fn         {function_id}
::index create cfg_assign:by_fn       {function_id}
::index create cfg_call:by_fn         {function_id}
::index create cfg_guard:by_fn        {function_id}
::index create cfg_resource_acquire:by_fn  {function_id}
::index create cfg_resource_release:by_fn  {function_id}
```

### Metadata

```cozoscript
:create build_meta       {key: String => value: String}
:create build_meta_files {file_path: String =>
                          hash: String, size: Int, mtime: Int}
```

## Built-in query templates

Small library shipped in `src/queries/builtin/`. Users invoke with
`--template <name> --param k=v`. All user values bound via `$param`, never
interpolated into the script text.

### `find_callers.cozoql`
```cozoscript
caller[c, 1] := *edge_calls{caller_id: c, callee_id: t},
                *symbol{id: t, name: $target}
caller[c, d + 1] := caller[m, d], d < $max_depth,
                    *edge_calls{caller_id: c, callee_id: m}
?[caller_name, caller_file, depth] :=
    caller[c, depth], *symbol{id: c, name: caller_name, file_path: caller_file}
:order depth, caller_file, caller_name
```

### `find_cycles.cozoql`
```cozoscript
reach[a, b] := *edge_imports{from_path: a, to_path: b}
reach[a, c] := reach[a, b], *edge_imports{from_path: b, to_path: c}
?[file, partner] := reach[file, partner], reach[partner, file], file < partner
```

### `complexity_hotspots.cozoql`
```cozoscript
?[name, file, line, complexity, length, severity, pattern, message] :=
    *symbol{id, name, file_path: file, start_line: line, kind},
    kind in ['function', 'method'],
    *metric_cyclomatic_complexity{symbol_id: id, value: complexity},
    *metric_function_length{symbol_id: id, value: length},
    *file_classification{path: file, is_test: false},
    complexity >= $cc_threshold,
    length >= $length_threshold,
    severity = if(complexity >= 20, 'error', if(complexity >= 10, 'warning', 'info')),
    pattern = 'high_complexity',
    message = format('{}:{} has CC={}, length={}', file, line, complexity, length)
:order complexity desc
```

The five-column convention `(file, line, severity, pattern, message)` —
plus any extras — is the CLI's signal to format output as audit findings.

### Other templates we ship
- `find_callees.cozoql` — outbound reachability with depth
- `find_function_by_name.cozoql` — name → symbols
- `export_surface.cozoql` — per-file ratio of exported vs total symbols
- `unused_symbols.cozoql` — symbols with no inbound `edge_calls`/`edge_imports`
- `taint_paths.cozoql` — source-to-sink reachability through CFG
- `unreleased_resources.cozoql` — `cfg_resource_acquire` not matched on every exit path
- `import_depth.cozoql` — longest import chain per file

Total: ~10 templates, each 5–30 lines. Authoritative examples for users.

## Build phase architecture

Parse workers run unchanged; the absorber now writes Cozo facts instead of
populating an in-memory graph.

```
parse workers (rayon, parallel)
   ├─ tree-sitter parse
   ├─ extract symbols / imports / call sites
   ├─ CfgFactBuilder per function (write-only accumulator)
   ├─ tree-sitter metric pass per function
   │     → cyclomatic / cognitive / nesting / length values
   ├─ classify file (is_test / is_barrel / is_generated)
   └─ extract nolint comments
                              │
                              ▼
       FileGraphData {symbols, imports, callsites,
                      cfg_facts: CfgFactBuilder,
                      metrics: PerFunctionMetrics,
                      file_class: FileClass,
                      nolints: Vec<NolintFact>}
                              │
                              ▼
            mpsc::sync_channel(2 × num_cpus)
                              │
                              ▼
   ┌─────────────────────────────────────────────┐
   │ single absorber thread (writes to Cozo)     │
   │   • assigns monotonic Int IDs                │
   │   • batches into ~10k-row transactions       │
   │   • writes all relations in one DB           │
   └─────────────────────────────────────────────┘
                              │
                              ▼
              cross-file edge resolution
              (deferred imports + calls)
                              │
                              ▼
              write build_meta + file hashes
```

## Phased migration

| # | Phase | Effort | Acceptance |
|---|---|---|---|
| 1 | **Bounded channel** (independent memory win) | 1-2 days | `mpsc::sync_channel(N)` in builder; openclaw build peak footprint drops |
| 2 | **Cozo dep + schema + write path (cross-function graph)** | 1 week | DB built from workspace; cross-function relations populated; `--cozo-only` test flag that runs trivial queries against the DB |
| 3 | **`CfgFactBuilder` + 9 language CFG builders rewritten** | 2 weeks | Language builders emit facts; CFG relations populated; per-language fact-shape tests |
| 4 | **Tree-sitter metrics computed at build time, written as facts** | 3-4 days | `metric_*` relations populated; basic sanity queries pass |
| 5 | **`projects query --cozoscript / --file / --template / --param`** | 1 week | New query interface live; built-in templates ship; output convention `(file, line, severity, pattern, message)` formats as findings |
| 6 | **THE DELETION PR** | 2-3 days | `rm -rf src/pipeline src/audit src/graph/taint src/graph/resource.rs src/graph/cfg.rs`; remove `audit` subcommand from CLI; remove JSON DSL parsing in query subcommand; **`cargo test` green** |
| 7 | **RocksDB persistence** | 3-4 days | Build writes to `~/.cache/virgil/<hash>.cozo/`; warm-start opens existing; first query < 1s on openclaw |
| 8 | **Incremental updates** | 1 week | `git pull && projects query` re-parses only changed files; cascade-delete facts for removed files |
| 9 | **Drop petgraph** | 1 day | `cargo tree | grep petgraph` empty |
| 10 | **Adopt Cozo `graph-algo` built-ins + new analyses** | 1 week | See "Phase 10 detail" below |

**Estimated total: 7-9 weeks.**

### Phase 10 detail — graph-algo built-ins

Cozo's `graph-algo` feature ships parallel implementations (Rayon-backed)
of every traversal algorithm we hand-rolled, plus several we couldn't
practically express in the legacy DSL. Phase 10 swaps templates over to
these built-ins **and** ships new templates that expose analyses
virgil-cli has never had.

**Direct swap-ins** — existing templates rewritten to use built-ins.
Hand-rolled Datalog recursion in these templates becomes a one-line
algorithm invocation:

| Template | Built-in it now uses | Notes |
|---|---|---|
| `find_cycles.cozoql` | `ConnectedComponents(strong: true)` | Replaces hand-rolled `reach[a,b]` recursion. Parallel, cheaper for large graphs. |
| `import_depth.cozoql` | `TopSort` + max-aggregation | DAG-aware longest path |
| `find_callers.cozoql` | `BFS` with `direction: backward` | Replaces depth-bounded recursive Datalog |
| `find_callees.cozoql` | `BFS` with `direction: forward` | Same |

**New templates** — analyses we couldn't ship with the legacy DSL:

| Template | Built-in used | What it answers |
|---|---|---|
| `shortest_call_chain.cozoql` | `ShortestPathBFS` | "What's the shortest call chain from `entry_function` to `dangerous_sink`?" Critical for security review. |
| `k_alternative_paths.cozoql` | `KShortestPathYen` | "Show me the top-K distinct paths from A to B." For "ways untrusted input can reach this sink." |
| `function_pagerank.cozoql` | `PageRank` | "Rank functions by structural importance in the call graph." Identifies hot spots that aren't obvious from line count or complexity. |
| `bridge_functions.cozoql` | `BetweennessCentrality` | "Which functions sit on the most call paths between others?" Refactoring targets — removing/changing them affects the most code. |
| `module_clusters.cozoql` | `CommunityDetectionLouvain` | "What are the natural subsystem boundaries based on call patterns?" Useful for "this codebase has organically grown — what would a clean module split look like?" |
| `import_pagerank.cozoql` | `PageRank` over `edge_imports` | "Which files are imported most centrally?" Identifies foundation modules. |
| `unreachable_from_main.cozoql` | `BFS` (negation against result) | "Functions not transitively reachable from any entry point." Dead code at architectural scale. |

**Why this is its own phase, not merged into earlier work**

- The earlier templates can be written *without* the built-ins (just
  recursive Datalog). Shipping a working query language first is more
  important than shipping the best implementations.
- The new templates unlock analyses virgil-cli hasn't had — they need
  documentation, examples, and probably their own announcement.
- The built-ins have their own performance characteristics (e.g. Cozo's
  `PageRank` requires a fully-loaded graph in memory; doesn't stream).
  Benchmarking on openclaw is part of the acceptance.

**Phase 10 acceptance**

- All applicable hand-rolled traversals in built-in templates swapped to
  built-ins.
- 7 new templates shipped (the ones above).
- Each new template documented with 1-2 worked examples.
- openclaw benchmark: each new template completes in < 10s on a warm
  cache.
- No new audit query types added to the CLI — these are all just
  `--template <name> --param ...` invocations.

### Sequencing notes

- **No `GraphStore` trait.** The previous plan included one as a firewall
  between backends. Now there is no second backend — Cozo is the *only*
  store after PR 6. The trait would be ceremony for nothing.
- **No equivalence-test phase.** The old DSL is being deleted, not
  matched against. Tests focus on Cozoscript queries producing expected
  rows on small fixture corpora.
- **PR 6 is the catharsis.** Single huge deletion commit; everything
  before it is additive, everything after refines what remains.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Cozoscript injection (user query strings) | Inline `--cozoscript` runs whatever the user passes (it's their CLI). Built-in templates use only `$param` binding; relation/column names from closed Rust enums. Document that `--cozoscript` is power-user mode. |
| Datalog perf for per-function analyses | PR 4 acceptance gate: representative analyses (taint, resource) ≤ 3× current Rust impl on openclaw. Function-scoped indices on all `cfg_*` relations. If perf misses, in-memory Cozo per function as fallback. |
| Build throughput regression | Cozo has one writer. PR 7 acceptance: ≤ 2× current build wall time on openclaw. RocksDB batch sizes tunable. |
| Cozo bus factor (one maintainer) | Less protected than the previous plan since `GraphStore` trait is gone. Mitigation: schema versioning + Cozo's `:export`/`:import` for data portability — we can dump to JSON-lines and reload into KuzuDB or another Datalog engine if Cozo dies. |
| User workflows depending on `audit` subcommand or built-in JSON pipelines | Breaking change. Mitigation: ship a release note + migration guide showing `projects query --template` equivalents for the most-used built-in audits. |
| Audit authors don't know Cozoscript | Templates + docs. Ship 10-20 worked examples covering 90% of cases. Cozoscript is small (~1 day to learn the subset we use). |
| Loss of stage composability (chain multiple stages) | A single Cozoscript query expresses what multi-stage pipelines did. Stratified rules let users derive intermediate concepts in one file. |
| Schema migrations across virgil-cli versions | `build_meta` stores `schema_version: Int`. Mismatch → rebuild from scratch (acceptable while schema churns; switch to migration chain once it's stable). |
| MPL-2.0 license | File-level copyleft. Using Cozo as a dependency is fine for OSS and proprietary alike. |
| Multi-process safety for `serve` | RocksDB allows one writer + many readers across processes. CLI build takes exclusive write lock; `serve` opens read-only. |

## Open questions

1. **Inline `--cozoscript` vs templates only.** Both? Templates-only is
   safer (no injection surface) but power-user-hostile. Both is right;
   document `--cozoscript` as power-user mode.
2. **SQLite vs RocksDB default backend?** RocksDB faster, C++ dep.
   SQLite pure Rust (via `sqlite3-src` bundled). Decision deferred to PR 7
   — benchmark both on openclaw.
3. **How does `match_pattern` (tree-sitter queries) compose with
   Cozoscript?** Two options: (a) emit a fixed set of tree-sitter results
   as facts at build time; (b) keep `match_pattern` as a separate `virgil-cli
   match` subcommand that runs tree-sitter and outputs file/line/text
   without touching Cozo. Recommend (b) — these are different concerns and
   shouldn't be composed in one query.
4. **Per-function in-memory Cozo for taint/resource if persistent perf
   misses gates?** Same fallback as the previous plan — ephemeral `mem`
   backend per function. Investigated during PR 4.
5. **Versioning the schema vs versioning the templates.** Templates
   reference relations by name. If we rename a relation we break every
   user's stored queries. Decision: relation names are part of the
   stable public API after PR 5 ships; schema changes go through a
   deprecation cycle.

## What success looks like

- `cargo tree | grep petgraph` returns nothing.
- `src/pipeline/`, `src/audit/`, `src/graph/taint/`, `src/graph/resource.rs`
  no longer exist.
- `virgil-cli projects query openclaw --template find_cycles` returns
  cycles in < 1s on a warm cache.
- `virgil-cli projects query openclaw --file my_audit.cozoql` runs
  arbitrary user audits.
- `du -sh ~/.cache/virgil/openclaw.cozo/`: 200-500 MB on openclaw.
- `cargo test`: green; tests cover schema, fact emission, and core
  templates on fixture corpora.
- Linux kernel as a corpus: `projects query` succeeds (slow but correct).
- Net Rust LOC change: **-22,000 / +3,000**.
- The `query` subcommand is the only way to ask the tool questions
  (besides registry CRUD and raw file reads).

## Execution order — TL;DR

1. **PR 1** — Bounded channel + absorber tighten. Memory win, independent.
2. **PR 2** — Cozo dep + schema + write path for cross-function graph.
3. **PR 3** — `CfgFactBuilder` + 9 language CFG builders rewritten;
   CFG facts in Cozo.
4. **PR 4** — Tree-sitter metrics → `metric_*` facts at build time.
5. **PR 5** — New `projects query` interface (`--cozoscript`, `--file`,
   `--template`, `--param`); ship `src/queries/builtin/*.cozoql`.
6. **PR 6** — **The Deletion.** `rm -rf src/pipeline src/audit
   src/graph/taint src/graph/resource.rs src/graph/cfg.rs`; remove
   `audit` subcommand; remove JSON DSL parsing in query subcommand.
7. **PR 7** — RocksDB persistence + cache-dir story.
8. **PR 8** — Incremental refresh.
9. **PR 9** — Drop petgraph from `Cargo.toml`.
10. **PR 10** — Adopt Cozo `graph-algo` built-ins; ship 7 new templates
    (`shortest_call_chain`, `k_alternative_paths`, `function_pagerank`,
    `bridge_functions`, `module_clusters`, `import_pagerank`,
    `unreachable_from_main`) that unlock analyses the legacy DSL
    couldn't express.

PR 6 is the centerpiece — the deletion. PR 10 is the celebration —
analyses the tool has never had before.
