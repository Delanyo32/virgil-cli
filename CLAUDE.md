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
cargo run -- projects query myapp --file query.cozoql --param target=x

# Force a cold rebuild of the persisted fact store
cargo run -- projects query myapp --template find_cycles --rebuild

# S3/R2 (no registration needed)
cargo run -- projects query --s3 s3://bucket/prefix --template find_cycles [--lang rs]

# Serve mode (HTTP). Routes: GET /health, POST /query {cozoscript|template, params}
cargo run -- serve --s3 s3://bucket/prefix [--host 127.0.0.1] [--port 0] [--lang rs]
```

## Module Layout

- `src/cozo/` — fact store wrapper
  - `schema.rs` — `:create` statements for the cross-function graph + `file_classification` + `nolint`
  - `store.rs` — `CozoStore` thin wrapper over `cozo::DbInstance` (SQLite-backed via `cache_dir_for`)
  - `writer.rs` — `CozoWriter` batched row accumulator (~10k rows per transaction); streamed during `GraphBuilder::build`
  - `from_code_graph.rs` — tail of the build pipeline; emits `comment` rows and the type/inheritance/throws/field_type rows that still need cross-file symbol-id resolution
- `src/queries/` — user-facing query surface
  - `runner.rs` — `run(QueryRequest)`: loads/dispatches, detects audit-shape output
  - `templates.rs` — embeds `builtin/*.cozoql` via `include_dir`
  - `rust_templates.rs` — handlers that need source access (currently only `complexity_hotspots`; `taint_paths`/`unreleased_resources` deferred until replacement CFG infra lands)
  - `builtin/*.cozoql` — 7 pure-Cozoscript templates (find_callers/callees/cycles, find_function_by_name, find_implementations_of, export_surface, import_depth)
- `src/graph/` — build-time scratch state
  - `mod.rs` — `CodeGraph` — a slim build-time scratch struct (interner + `symbol_ids_by_name` lookup + per-file type/comment/inheritance buckets). No node Vec, no adjacency lists — `*file`/`*symbol`/`*span`/`*calls`/`*imports` rows stream straight to Cozo during absorb.
  - `builder.rs` — `GraphBuilder` (parses workspace into the slim graph + streams rows); `find_node_at_line` used by `complexity_hotspots`
  - `metrics.rs` — metric computation (cyclomatic complexity, function length, etc.) — called on-demand from `rust_templates::complexity_hotspots`
- `src/classify.rs` — `is_test_file`, `is_barrel_file` — used at build time to populate `file_classification` facts
- `src/languages/` — one deep module per language, plus shared facade
  - `mod.rs` — language-agnostic facade (`compile_*_query`, `extract_*`, `resolve_import`)
  - `<lang>/{queries.rs, mod.rs}` — per-language tree-sitter queries + extractors

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
- `CodeGraph` lives only during a cold/incremental build; it's not shared with queries. Queries hit the `CozoStore` directly.

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

**CallSite nodes are materialised during absorption**
`NodeWeight::CallSite` is emitted for *every* call expression encountered inside a symbol's line range. The node carries `arg_literals` (string/number/bool literals only), `enclosing_test_name` (set when `is_test_file(path) && is_test_function_name(symbol)`), and `caller_symbol`. A `Contains` edge from the caller `Symbol` to the `CallSite` is added. Literal extraction uses a single union node-kind whitelist across grammars (`is_literal_kind` in `builder.rs`); per-language refinement is a follow-up.

**Streaming graph builder**
`GraphBuilder::build` parses files on rayon workers and sends each `FileGraphData` over a bounded `mpsc::sync_channel(2 * num_cpus)` to a single-threaded drainer (`absorb_file_data` in `builder.rs`). Per-file work — File/Symbol/CallSite nodes plus DefinedIn/Contains/Exports edges — happens during absorption. Cross-file refs (`Imports`, `Calls` edges) are queued in `DeferredImport`/`DeferredCall` tuples and resolved after the channel drains.

**Local workspace is disk-backed**
`Workspace::load` no longer reads file contents up front. It records sizes + language extensions only; `DiskFileSource` (`src/storage/file_source.rs`) reads on demand and caches in a small LRU (`lru` crate, cap 256). S3 workspaces still use the in-memory `MemoryFileSource` since fetches are batched at startup.

**S3 workspace**
- S3 workspace root is a synthetic `s3://bucket/prefix` path.
- `--s3` flag conflicts with positional `name`/`dir` args via `#[arg(conflicts_with)]`
- Server mode (`serve`) is S3-only — no `--path` flag. Used by Virgil Live (cloud service).

**Cozo store lifecycle**
The query pipeline builds the `CodeGraph` first, then calls `cozo::populate(&store, &graph, Some(&workspace))` to materialise the cross-function relations + `file_classification` + `nolint` into an in-memory `CozoStore`. `NodeIndex::index()` is used as the monotonic `Int` id for `symbol`/`callsite` rows. On-disk persistence is a separate follow-up issue.

**Rust-side template, not pure Cozoscript**
`complexity_hotspots` lives in `src/queries/rust_templates.rs`. It escapes Cozoscript because metrics aren't materialised as facts — the handler queries `*symbol` + `*span` + `*file_classification` from Cozo, then calls `graph::metrics::compute_*` on demand for each function. All other built-in templates are pure Cozoscript.

**`throws` extraction is not uniform across languages**
Java extracts the declared `throws` clause on method/constructor declarations. C# and PHP have no declared throws keyword — `extract_throws` walks `throw_statement` / `throw_expression` nodes and pulls the exception type out of `throw new X(...)` forms only. Re-throws and variable re-raise (`throw e;`) have no static type and emit no row. Other 6 languages return an empty `Vec<ThrowsRow>`. `from_code_graph::emit_types_and_hierarchy` synthesises a `*type{kind: "named"}` row when an exception type wasn't already seen by `extract_types`, so the 3-way JOIN through `*type` succeeds.

**Python class-body assignments emit `Field` symbols**
`class C: x: int = 5` (and untyped `x = 5`) produce a `kind=field` `Symbol` row in addition to whatever the type extractor emits. This is what makes `*symbol{kind: "field"} JOIN *field_type` non-empty.

**Persistence + warm-start**
`CozoStore::open_persistent(path)` opens a SQLite-backed Cozo store at `~/.cache/virgil/<hash>.cozo`. `cache_dir_for(id)` derives the hash via FNV-1a from the project name (or S3 URI). On open: if the file exists and `build_meta.schema_version` matches the compiled-in `SCHEMA_VERSION`, the store reopens warm; otherwise the file is removed and a fresh schema applied. `cozo::workspace_diff(&store, &workspace)` returns the `(added, modified, removed)` diff against `build_meta_files`; `cozo::incremental_refresh` re-parses only touched files and re-resolves cross-file edges from facts. Cold ~850ms, warm ~17ms on the reference workspace.

**Schema-version bumps**
`SCHEMA_VERSION` in `src/cozo/mod.rs` lives next to the `:create` statements. Bump it whenever the shape of `schema::create_statements()` or `index_statements()` changes — the open path will detect mismatch and wipe stale stores automatically. Currently at `6` (removed the eager `references` relation and its resolver).

**No eager reference resolution**
The build path emits the raw facts only — `occurrence`, `scope`, `binding`, `imports`, `symbol`, and so on. There is no `*references` relation and no built-in resolver. The earlier staged Cozoscript resolver materialised that relation eagerly at build end; on big repos its `rsv_ancestor` transitive-closure stage dominated build memory + time (django: 4.6 GB / 5.8 min vs 465 MB / 12 s without it; 5.5k-file repos went from OOM to ~600 MB / 27 s). Removing it shifted the cost: callers that want resolved references write their own Cozoscript over the raw facts at query time, scoped to whatever demand set they actually need. See `docs/resolution.md` for the staged-resolver algorithm if you want to port it back into a per-query template.

**Cozoscript rule-head gotcha**
Cozo does not accept literals in rule-head positions. `caller[c, 1] := ...` fails to parse; bind the constant in the body instead: `caller[c, d] := d = 1, ...`. All recursive built-in templates use this pattern.

**Cozo silently drops relations whose names start with `_`**
A `:replace _foo {...}` or `:create _foo {...}` returns `Ok(status: OK)` but the relation is never actually materialised; subsequent `*_foo{}` reads fail with `Cannot find requested stored relation '_foo'`. Don't add new `_`-prefixed names — prefix scratch relations with `rsv_` or similar.

<!-- GSD:skills-start source:skills/ -->
## Project Skills

| Skill | Description | Path |
|-------|-------------|------|
| grill-me | Interview the user relentlessly about a plan or design until reaching shared understanding, resolving each branch of the decision tree. Use when user wants to stress-test a plan, get grilled on their design, or mentions "grill me". | `.agents/skills/grill-me/SKILL.md` |
| skill-creator | Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Claude's capabilities with specialized knowledge, workflows, or tool integrations. | `.agents/skills/skill-creator/SKILL.md` |
| virgil | > Explore and query codebases using virgil-cli. Covers project registration, Cozoscript templates for symbol search and call graph traversal, file reading. Use when asked to analyze a codebase, understand architecture, find symbols, trace callers/callees, onboard to a project, investigate bugs, or map the API surface of any TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebase. | `.agents/skills/virgil/SKILL.md` |
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