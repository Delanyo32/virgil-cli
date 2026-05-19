# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases on-demand and exposes them as CozoDB relations queryable via Cozoscript. No database to manage — projects are registered by name and parsed at query time into an in-memory Cozo store. Supports S3-compatible storage (AWS S3, Cloudflare R2, MinIO) via `--s3 s3://bucket/prefix`.

## Build & Run

```bash
cargo build
cargo run -- projects create myapp --path ./src [--lang ts,tsx,js,jsx] [--exclude "vendor/**"]
cargo run -- projects list
cargo run -- projects delete myapp

# Query — exactly one of --cozoscript / --file / --template required
cargo run -- projects query myapp --template find_function_by_name --param name=login
cargo run -- projects query myapp --cozoscript '?[name] := *symbol{name}'
cargo run -- projects query myapp --file audit.cozoql --param target=x

# S3/R2 (no registration needed)
cargo run -- projects query --s3 s3://bucket/prefix --template find_cycles [--lang rs]

# Serve mode (HTTP). Routes: GET /health, POST /query {cozoscript|template, params}
cargo run -- serve --s3 s3://bucket/prefix [--host 127.0.0.1] [--port 0] [--lang rs]

# Skip expensive passes (memory/CPU savings — templates depending on
# CFG/resource analysis silently produce no findings):
cargo run -- serve --s3 s3://bucket/prefix --no-cfg
cargo run -- serve --s3 s3://bucket/prefix --no-resource-graph
cargo run -- serve --s3 s3://bucket/prefix --symbols-only
```

## Module Layout

- `src/cozo/` — fact store wrapper
  - `schema.rs` — `:create` statements for the cross-function graph + `file_classification` + `nolint`
  - `store.rs` — `CozoStore` thin wrapper over `cozo::DbInstance` (in-memory; on-disk lands in the persistence issue)
  - `writer.rs` — `CozoWriter` batched row accumulator (~10k rows per transaction)
  - `from_code_graph.rs` — walks a finished `CodeGraph` and emits the equivalent facts
- `src/queries/` — user-facing query surface
  - `runner.rs` — `run(QueryRequest)`: loads/dispatches, detects audit-shape output
  - `templates.rs` — embeds `builtin/*.cozoql` via `include_dir`
  - `rust_templates.rs` — handlers that need source access (complexity_hotspots, taint_paths, unreleased_resources)
  - `builtin/*.cozoql` — 7 pure-Cozoscript templates (find_callers/callees/cycles, find_function_by_name, export_surface, import_depth, unused_symbols)
- `src/graph/` — graph data structures and builder
  - `mod.rs` — `CodeGraph`, `NodeWeight`, `EdgeWeight`, `GraphNode`
  - `builder.rs` — `GraphBuilder` (parses workspace into `CodeGraph`)
  - `taint/` — `TaintEngine`, `TaintConfig` and pattern types (consumed by the Rust-side `taint_paths` template handler)
  - `metrics.rs` — metric computation (cyclomatic complexity, function length, etc.) — called on-demand from `rust_templates::complexity_hotspots`
  - `resource.rs` — `ResourceAnalyzer` (acquires/released_by edges; consumed by the Rust-side `unreleased_resources` handler)
  - `cfg.rs` — control flow graph data structures (used by taint + resource analyses)
- `src/classify.rs` — `is_test_file`, `is_barrel_file` — used at build time to populate `file_classification` facts
- `src/languages/` — one deep module per language, plus shared facade
  - `mod.rs` — language-agnostic facade (`compile_*_query`, `extract_*`, `resolve_import`)
  - `cfg.rs` — `CfgBuilder` trait and `cfg_builder_for_language` dispatch
  - `<lang>/{queries.rs, cfg.rs, mod.rs}` — per-language: tree-sitter queries+extractors and CFG builder, one folder per language

## Cozoscript query surface

The `projects query` subcommand accepts Cozoscript via three flags
(mutually exclusive):

- `--template <name>` — a built-in template under `src/queries/builtin/`
- `--cozoscript '<inline>'` — power-user mode
- `--file <path>` — load Cozoscript from a file

User-supplied values bind via `--param key=value` (repeatable). Integers
and booleans are auto-coerced; everything else binds as a string.
Parameters reach Cozoscript through `BTreeMap<String, DataValue>` — never
through string interpolation.

**Audit-shape convention:** a query (or template handler) that returns
columns `(file, line, severity, pattern, message)` is auto-formatted as
audit findings instead of a raw row table. Extra columns are preserved.

## Non-obvious Implementation Notes

Critical gotchas and design decisions that are not obvious from reading the code:

**tree-sitter 0.25 API (do not downgrade)**
`QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`. Downgrading tree-sitter breaks this.

**Threading constraints**
- `tree_sitter::Parser` is `!Send` — create a fresh instance per rayon task (never share or pool)
- `tree_sitter::Query` objects are `Arc`-shareable — compile once per language, share across threads
- `CodeGraph` (`petgraph::DiGraph`) is `Send` but not `Sync` — share via `Arc<CodeGraph>` for read-only access. In `serve`, `AppState` wraps it in `Arc<RwLock<CodeGraph>>` so the lazy resource pass can take a brief write lock; queries hold the read lock.

**File extension mapping**
- `.h` files map to C (deliberate design choice). C++ headers must use `.hpp`/`.hxx`/`.hh`
- PHP grammar uses `LANGUAGE_PHP` (handles `<?php` tags), not `LANGUAGE_PHP_ONLY`

**Query behavior quirks**
- `find: "function"` matches `Function` AND `ArrowFunction` — both are returned
- `find: "constructor"` matches `Method` kind filtered by name (not a separate kind)
- `inside` filter uses line-range containment, not AST hierarchy
- `read` field bypasses the entire symbol extraction pipeline — returns raw content
- `has: {"not": "docstring"}` finds symbols *without* associated doc comments (inverse match)
- Import `kind` is a free-form `String` (not an enum) so language modules can add new kinds without changing `models.rs`

**Python parsing**
- `decorated_definition` nodes: unwrap to inner function/class; skip the bare `function_definition`/`class_definition` if its parent is a `decorated_definition`. This deduplication prevents double-reporting decorated symbols.

**Call graph**
Name-based resolution via `symbols_by_name` lookup — heuristic only, no type info. BFS with configurable depth (max 5). Replaces old `call_graph.rs`.

**CallSite nodes are materialised at builder step 5d**
`NodeWeight::CallSite` is emitted for *every* call expression encountered inside a symbol's line range — not just resource-tracking calls. The node carries `arg_literals` (string/number/bool literals only), `enclosing_test_name` (set when `is_test_file(path) && is_test_function_name(symbol)`), and `caller_symbol`. A `Contains` edge from the caller `Symbol` to the `CallSite` is added. Resource analysis still creates synthetic `acquire:<resource>` CallSites at `src/graph/resource.rs:95` — those have empty `arg_literals` and `caller_symbol = Some(function_node)`. Literal extraction uses a single union node-kind whitelist across grammars (`is_literal_kind` in `builder.rs`); per-language refinement is a follow-up.

**CfgExit nodes + `ExitsVia` edges (builder step 5e)**
For every entry in `FunctionCfg.exits`, the builder emits one `NodeWeight::CfgExit` and a `Contains` edge plus an `EdgeWeight::ExitsVia(CfgExitKind)` edge from the symbol. `CfgExitKind` is picked from the inbound CFG edge with priority `Exception > Cleanup > TrueBranch > FalseBranch > Normal` (`classify_cfg_exit` in `builder.rs`). `exit_label`: branch-kind exits join the originating `Guard.condition_vars` with ` & `; exception-kind exits use the first `Call.name` in the exit block; normal/cleanup are `None`. The DSL exposes the edge as five flat `EdgeType` variants (`exits_via_normal/true/false/exception/cleanup`) — `EdgeType` itself is still a payload-free flat enum.

**CFGs are not retained — they are rebuilt on demand**
`CodeGraph` no longer carries a `function_cfgs: HashMap<NodeIndex, FunctionCfg>` map. Instead it exposes `function_cfg_indices: HashSet<NodeIndex>` (the function symbols whose CFGs *could* be built) plus a private `Mutex<HashMap<NodeIndex, FunctionCfg>>` lazy cache. `CodeGraph::cfg_for_function(workspace, idx)` re-parses the function's source and builds the CFG on demand, then caches it. Tests inject synthetic CFGs via `inject_cfg(idx, cfg)` and pass `workspace = None` to `TaintEngine::analyze_all` / `ResourceAnalyzer::analyze_all`. Production callers pass `Some(workspace)`.

**Resource analysis is lazy**
`GraphBuilder::build()` no longer calls `ResourceAnalyzer::analyze_all`. Call `graph.ensure_resource_graph(Some(&workspace))` (idempotent) to populate `Acquires`/`ReleasedBy` edges. Callers that need lifecycle edges trigger this explicitly; query handlers that don't touch resources skip it entirely.

**Streaming graph builder**
`GraphBuilder::build` parses files on rayon workers and sends each `FileGraphData` over a bounded `mpsc::sync_channel(2 * num_cpus)` to a single-threaded drainer (`absorb_file_data` in `builder.rs`). Per-file work — File/Symbol/CallSite nodes, `ExitsVia` edges, dropping the CFG — happens during absorption. Cross-file refs (`Imports`, `Calls` edges) are queued in `DeferredImport`/`DeferredCall` tuples and resolved after the channel drains.

**`BuildOptions` + skip flags**
`GraphBuilder::with_options(BuildOptions { build_cfgs, build_resource_graph })` gates pass-by-pass work. `--no-cfg` clears `build_cfgs` (CFGs are not built at all, so `function_cfg_indices` stays empty and there are no `ExitsVia` edges, taint findings, or lifecycle findings). `--no-resource-graph` clears `build_resource_graph` (suppresses `ensure_resource_graph`). `--symbols-only` is shorthand for both. Templates that depend on suppressed edges silently return no findings.

**Local workspace is disk-backed**
`Workspace::load` no longer reads file contents up front. It records sizes + language extensions only; `DiskFileSource` (`src/storage/file_source.rs`) reads on demand and caches in a small LRU (`lru` crate, cap 256). S3 workspaces still use the in-memory `MemoryFileSource` since fetches are batched at startup.

**S3 workspace**
- S3 workspace root is a synthetic `s3://bucket/prefix` path.
- `--s3` flag conflicts with positional `name`/`dir` args via `#[arg(conflicts_with)]`
- Server mode (`serve`) is S3-only — no `--path` flag. Used by Virgil Live (cloud service).

**Cozo store lifecycle**
The query pipeline builds the `CodeGraph` first, then calls `cozo::populate(&store, &graph, Some(&workspace))` to materialise the cross-function relations + `file_classification` + `nolint` into an in-memory `CozoStore`. `NodeIndex::index()` is used as the monotonic `Int` id for `symbol`/`callsite` rows. On-disk persistence is a separate follow-up issue.

**Three Rust-side templates, not pure Cozoscript**
`complexity_hotspots`, `taint_paths`, and `unreleased_resources` live in `src/queries/rust_templates.rs`. They escape Cozoscript because:
- metrics aren't materialised as facts (re-derivable via `graph::metrics::compute_*`)
- CFG isn't materialised as facts (re-derivable via `CodeGraph::cfg_for_function`)
- `taint_paths` / `unreleased_resources` currently return empty findings; wiring them into `src/graph/taint/` + `src/graph/resource.rs` is a follow-up.

**Cozoscript rule-head gotcha**
Cozo does not accept literals in rule-head positions. `caller[c, 1] := ...` fails to parse; bind the constant in the body instead: `caller[c, d] := d = 1, ...`. All recursive built-in templates use this pattern.

<!-- GSD:skills-start source:skills/ -->
## Project Skills

| Skill | Description | Path |
|-------|-------------|------|
| grill-me | Interview the user relentlessly about a plan or design until reaching shared understanding, resolving each branch of the decision tree. Use when user wants to stress-test a plan, get grilled on their design, or mentions "grill me". | `.agents/skills/grill-me/SKILL.md` |
| skill-creator | Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Claude's capabilities with specialized knowledge, workflows, or tool integrations. | `.agents/skills/skill-creator/SKILL.md` |
| virgil | > Explore and query codebases using virgil-cli. Covers project registration, JSON query language for symbol search, call graph traversal, file reading, and static audit. Use when asked to analyze a codebase, understand architecture, find symbols, trace callers/callees, onboard to a project, investigate bugs, or map the API surface of any TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebase. | `.agents/skills/virgil/SKILL.md` |
Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.