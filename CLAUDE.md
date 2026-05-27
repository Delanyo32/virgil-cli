# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases and exposes them as DuckDB tables queryable via SQL (with SQL/PGQ graph extensions via the `duckpgq` community extension). Projects are registered by name; first query parses the workspace and persists a file-backed DuckDB fact store at `~/.cache/virgil/<hash>.duckdb`. Subsequent queries warm-start in tens of milliseconds.

## Working notes for Claude

Lessons from prior sessions on this codebase. Read this before doing real work.

**Measure before theorizing.** "Where is the memory going?" — don't guess. Add a checkpoint print and run. One memory-checkpoint pass beat hours of speculation about buffer pools, reduce intermediates, etc. Same rule for "is X slow?" / "does the channel block?" — measure, don't reason from first principles.

**Verify before claiming.** Bench-printing a matching scalar count is a sniff test, not verification. Before saying "this works" / "tests pass" / "matches baseline":
- Run `cargo test --release` and read the output. Don't skip it because the change "feels small."
- For data-shape claims, diff every relevant table count between branches. Don't extrapolate from one `COUNT(*)`.

**Don't pattern-match from training data on infrastructure facts.** "DuckDB needs single-threaded writes" (wrong — MVCC), "the interner is single-threaded" (wrong — `ThreadedRodeo`), "periodic flush will halve RSS" (wrong — saved 10%). Fetch the docs or read the source. Past-sessions said these confidently and was wrong every time.

**Short answers for short questions.** "Why do we need X?" — two sentences. Not a table, not a "what this buys you" section. When the user types "simply put" or "caveman" or anything that signals brevity, that's a correction — match it on the next answer and stop reverting.

**Don't expand scope from a yes/no question.** "Can we do X?" → answer the question first, propose scope second, act after confirmation. "Let's bench this" is not authorisation for a multi-hour refactor.

**Surface regressions before merging.** A "fix" that makes the bench slower is a regression. Say so loudly, then let the user decide. Don't bury it under a tradeoff table.

**This codebase has specific known wrong claims to avoid repeating:**
- DuckDB supports concurrent writers via MVCC; `duckdb::Connection` is `Send` (not `Sync`) — use a pool, not "one connection per process."
- `lasso::ThreadedRodeo` (in `graph::intern::Symbols`) is thread-safe and `Arc`-shared; no need to design around it being single-threaded.
- The dominant memory term during parse+absorb is **per-worker scratch state in rayon's fold/reduce**, not `DbWriter` buffers, not DuckDB's buffer pool, not the `CodeGraph` HashMaps (those existed pre-refactor and have since been removed). The current shared-`Mutex<DbWriter>` design avoids it.

## Build & Run

```bash
cargo build
cargo run -- projects create myapp --path ./src [--lang ts,tsx,js,jsx] [--exclude "vendor/**"]
cargo run -- projects list
cargo run -- projects delete myapp

# Query — exactly one of --sql / --file / --template required
cargo run -- projects query myapp --template find_function_by_name --param name=login
cargo run -- projects query myapp --sql 'SELECT name FROM symbol LIMIT 10'
cargo run -- projects query myapp --file query.sql --param target=x

# Force a cold rebuild of the persisted fact store
cargo run -- projects query myapp --template find_cycles --rebuild
```

Local CLI only. `--s3` and `serve` were dropped during the DuckDB swap (see `docs/experiments/duckdb-swap.md`); S3 / cloud support is out of tree.

## Module Layout

- `src/db/` — fact store wrapper
  - `schema.rs` — `CREATE TABLE` / `CREATE INDEX` / `CREATE PROPERTY GRAPH codegraph` DDL. Includes the `raw_inheritance` staging table that absorb writes into and `resolve_inheritance` reads from (schema v1)
  - `store.rs` — `DbStore` thin wrapper over `duckdb::Connection`. Loads the duckpgq extension at open. Cache file at `~/.cache/virgil/<hash>.duckdb` via `cache_dir_for_db`
  - `writer.rs` — `DbWriter` batched row accumulator; on flush, opens a DuckDB `Appender` per non-empty table. The 9 `*_attrs` tables (VARCHAR[] columns) go through a batched literal `INSERT VALUES` path because duckdb 1.2's appender doesn't bind `Value::List`
  - `from_code_graph.rs` — post-parse populate phase. After the SQL-staging refactor it only runs the SQL `resolve_inheritance` (joins `raw_inheritance` ⨝ `symbol` ⨝ `imports` to emit `extends`/`implements`), `record_build_meta_files`, and `resolve_and_emit_call_edges` (rayon-parallel reads from `call_site` + `symbol` + `imports`). `comment` / `type` / `parameter` / `returns_type` / `field_type` / `throws` rows are emitted file-locally during absorb — this module no longer holds them
- `src/queries/` — user-facing query surface
  - `runner.rs` — `run(QueryRequest)`: loads/dispatches, detects audit-shape output
  - `templates.rs` — embeds `builtin/*.sql` via `include_dir`
  - `rust_templates.rs` — handlers that need source access (currently only `complexity_hotspots`)
  - `builtin/*.sql` — 7 templates (find_callers/callees/cycles/function_by_name/implementations_of/export_surface/import_depth). `find_cycles` and `import_depth` use recursive CTEs; the others are flat SQL joins
- `src/graph/` — build-time scratch state
  - `mod.rs` — `CodeGraph` — after the SQL-staging refactor this is just a thin wrapper around the shared `Symbols` interner. The per-file type/comment/inheritance HashMaps that used to live here are gone — workers now emit those rows directly to DuckDB (file-local resolution) or to the `raw_inheritance` staging table (cross-file resolution)
  - `builder.rs` — `GraphBuilder` (parses workspace + streams rows to DuckDB through a shared `Mutex<SharedAbsorb>`); `find_node_at_line` used by `complexity_hotspots`
  - `metrics.rs` — metric computation (cyclomatic complexity, function length, etc.) — called on-demand from `rust_templates::complexity_hotspots`
- `src/classify.rs` — `is_test_file`, `is_barrel_file` — used at build time to populate `file_classification` facts
- `src/languages/` — one deep module per language, plus shared facade
  - `mod.rs` — language-agnostic facade (`compile_*_query`, `extract_*`, `resolve_import`)
  - `<lang>/{queries.rs, mod.rs}` — per-language tree-sitter queries + extractors

## SQL query surface

The `projects query` subcommand accepts SQL via three flags (mutually exclusive):

- `--template <name>` — a built-in template under `src/queries/builtin/`
- `--sql '<inline>'` — power-user mode
- `--file <path>` — load SQL from a file

User-supplied values bind via `--param key=value` (repeatable). Integers and booleans are auto-coerced; everything else binds as a string. Parameters substitute into `$name` placeholders in the SQL as quoted literals — see "duckpgq gotchas" below for why we don't use prepared-statement binding.

**Audit-shape convention:** a query (or template handler) that returns columns `(file, line, severity, pattern, message)` is auto-formatted as audit findings instead of a raw row table. Extra columns are preserved.

**PGQ graph queries.** The schema includes `CREATE PROPERTY GRAPH codegraph` with two vertex tables (`file`, `symbol`) and four edge tables (`call_edge`, `imports`, `extends`, `implements`). Queries that want graph traversal use `SELECT ... FROM GRAPH_TABLE (codegraph MATCH ... COLUMNS (...))`. Currently `find_callers` and `find_callees` use PGQ for the single-hop join; `find_cycles` uses a recursive CTE because duckpgq 1.x crashes when `GRAPH_TABLE` is wrapped in a `WITH` clause.

## Non-obvious Implementation Notes

Critical gotchas and design decisions that are not obvious from reading the code:

**tree-sitter 0.25 API (do not downgrade)**
`QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`. Downgrading tree-sitter breaks this.

**Threading constraints**
- `tree_sitter::Parser` is `!Send` — create a fresh instance per rayon task (never share or pool)
- `tree_sitter::Query` objects are `Arc`-shareable — compile once per language, share across threads
- `CodeGraph` lives only during a cold build; it's not shared with queries. Queries hit the `DbStore` directly.

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
Name-based resolution scoped to the caller's imports (`file_symbols_by_name` for same-file matches; `file_exports_by_name` filtered by `file_imports` for cross-file). Heuristic only, no type info. Resolved during the post-absorb `DeferredCall` loop in `builder.rs`; emits `call_site` rows directly to DuckDB. The materialised `call_edge` table is then populated by `from_code_graph::resolve_and_emit_call_edges` so queries can join it directly.

**Pre-existing extractor bug (filed against master)**
`call_site.caller_id` resolves to the nearest parameter symbol instead of the enclosing function when the function takes parameters. Surfaces in `find_callers` / `find_callees` output as a wrong "caller" name. Independent of the DuckDB swap — the extractor lives in `graph/builder.rs` and was unchanged. Filed for follow-up.

**Shared-writer parallel graph builder**
`GraphBuilder::build` runs rayon `par_iter().try_for_each(...)` over the file list. Each worker parses a file lock-free (tree-sitter + extractors), then briefly takes a `Mutex<SharedAbsorb>` to push rows into a single shared `DbWriter` + the cross-file deferred Vecs + the interner. Periodic flush (`STREAM_FLUSH_EVERY_N_FILES`) caps writer memory. The critical section is short — Vec appends + a few HashMap inserts — so mutex contention doesn't dominate wall time.

Per-file resolution happens during absorb (file-local lookups via a local `name_to_id` map + per-file `type_id_by_display` map). Cross-file refs (`Imports`, inheritance, `Calls`) are either queued for a post-absorb Rust loop (`DeferredImport`/`DeferredCall`) or written to a DuckDB staging table (`raw_inheritance`) for SQL resolution. `CodeGraph` itself is now just a shared interner — the per-file HashMap buckets the old populate phase consumed have been deleted.

Earlier designs explored on this branch: (1) `mpsc::sync_channel` + single drainer thread (master) — wall 25.7s, RSS 860 MiB; (2) per-worker `WorkerLocal` with rayon `fold/reduce` — wall 16.5s but RSS 1.8 GiB; (3) shared-writer (current) — wall 28.8s, RSS 760 MiB. See `docs/experiments/duckdb-swap-findings.md` for the full matrix. We picked design 3 because the memory regression in 2 was structural to fold/reduce.

**Local workspace is disk-backed**
`Workspace::load` no longer reads file contents up front. It records sizes + language extensions only; `DiskFileSource` (`src/storage/file_source.rs`) reads on demand and caches in a small LRU (`lru` crate, cap 256).

**DbStore lifecycle**
The query pipeline opens (or creates) the file-backed `DbStore`, runs `GraphBuilder::build(&store)` which streams the full per-file fact set into DuckDB during absorb (`file`/`symbol`/`span`/`call_site`/`raw_import`/`*_attrs`/`scope`/`binding`/`occurrence` plus the file-locally-resolved `comment`/`type`/`parameter`/`returns_type`/`field_type`/`throws` rows, plus the unresolved `raw_inheritance` staging rows), then `db::populate(&store, &graph, Some(&workspace))` runs the post-parse phase: `resolve_inheritance` (SQL JOIN of `raw_inheritance` ⨝ `symbol` ⨝ `imports` with `ROW_NUMBER` priority to pick one parent per child), `record_build_meta_files`, and `resolve_and_emit_call_edges` (rayon-parallel — reads `call_site`/`symbol`/`imports` into Rust hash maps, emits `call_edge` rows). Symbol IDs are ADR-0002 stringly ids — `path|start_line|start_col|name|kind` — computed by `from_code_graph::symbol_id`.

**Rust-side template, not pure SQL**
`complexity_hotspots` lives in `src/queries/rust_templates.rs`. It escapes SQL because metrics aren't materialised as facts — the handler queries `symbol` + `span` + `file_classification` from DuckDB, then calls `graph::metrics::compute_*` on demand for each function. All other built-in templates are pure SQL.

**`throws` extraction is not uniform across languages**
Java extracts the declared `throws` clause on method/constructor declarations. C# and PHP have no declared throws keyword — `extract_throws` walks `throw_statement` / `throw_expression` nodes and pulls the exception type out of `throw new X(...)` forms only. Re-throws and variable re-raise (`throw e;`) have no static type and emit no row. Other 6 languages return an empty `Vec<ThrowsRow>`. `absorb_file_data` synthesises a `type{kind: "named"}` row inline when an exception type wasn't already seen by `extract_types` in the same file, so the 3-way JOIN through `type` succeeds.

**Python class-body assignments emit `Field` symbols**
`class C: x: int = 5` (and untyped `x = 5`) produce a `kind=field` `Symbol` row in addition to whatever the type extractor emits. This is what makes `symbol{kind: "field"} JOIN field_type` non-empty.

**`extends` / `implements` only reference resolved symbols**
The SQL `resolve_inheritance` resolver in `db/from_code_graph.rs` does an INNER JOIN to `symbol` for both endpoints — if a parent class lives outside the workspace (e.g. `class Foo extends Error` where `Error` is a TS built-in), the edge is dropped. The prior Rust resolver had a `parent_canonical_name` fallback that put a synthetic string like `typescript::global::Error` into `extends.parent_id`, breaking referential integrity with `symbol.id` (no row in `symbol` had that id). On the openclaw `extensions` corpus this drops 62 of 112 orphan rows from `extends`. The behaviour change is intentional: every `extends`/`implements` row now joins cleanly to `symbol`.

**Persistence + warm-start**
`DbStore::open_persistent(path)` opens a file-backed DuckDB store at `~/.cache/virgil/<hash>.duckdb`. `cache_dir_for_db(id)` derives the hash via FNV-1a from the project name. On open: if the file exists and `build_meta.schema_version` matches the compiled-in `SCHEMA_VERSION`, the store reopens warm; otherwise the file is removed and a fresh schema applied. Bench numbers from the swap (openclaw subset, see `docs/experiments/duckdb-swap-findings.md`):

- openclaw/discord (522 ts/tsx): cold parse 0.47s wall / 110 MB; warm queries ~0.28s (process-startup floor)
- openclaw/ui (461 ts/tsx): cold parse 0.45s wall / 129 MB; warm queries ~0.28s

Incremental refresh is intentionally NOT implemented — the experiment scope was cold + warm only. The `subset()` method on `Workspace` remains for future use.

**Schema-version bumps**
`SCHEMA_VERSION` in `src/db/mod.rs` lives next to the DDL statements. Bump it whenever the shape of `schema::create_statements()`, `index_statements()`, or `pgq_statements()` changes — the open path will detect mismatch and wipe stale stores automatically. Currently at `1` (fresh — no continuity with the prior Cozo schema versioning).

**duckpgq gotchas (1.x)**
- Vertex tables do **not** accept explicit `KEY (col)` clauses — the PK is taken implicitly from the table's primary key. Edge tables still need explicit `SOURCE KEY (...) DESTINATION KEY (...)`.
- Unbounded traversal (`->+` / `->*`) requires an explicit path mode like `ACYCLIC` — duckpgq rejects the default `WALK` because cycles would produce infinite results.
- `GRAPH_TABLE(...)` cannot be wrapped in a `WITH` CTE — triggers `INTERNAL Error: NULL unique_ptr`. Either inline the MATCH in the outer `FROM`, or fall back to a recursive CTE over the underlying `call_edge` table (which is what `find_cycles` does).
- `$name` placeholders inside `GRAPH_TABLE(... WHERE ...)` are not bound by DuckDB's prepared-statement layer — duckpgq parses the WHERE itself and the outer parameter binder never sees the `?`. We sidestep with literal substitution at runtime; trusted CLI input only (no injection threat model).
- `--` line comments + `/* */` block comments are stripped before parsing because duckpgq's parser rejects leading `--` on PGQ-flavored statements (plain DuckDB accepts them). Side benefit: comments containing `$name` doc references stop being treated as bindable placeholders.

**duckdb-rs gotchas (1.2)**
- `column_count` / `column_name` panic if called on a prepared-but-not-yet-queried statement — the schema isn't bound until execution. Snapshot column names from the first row's `as_ref()` after `query()` materialises the result set.
- `Appender::append_row` doesn't handle `Value::List` — `ValueRef::from(Value::List(..))` is `unimplemented!()`. The 9 `*_attrs` tables (VARCHAR[] columns) route through a batched literal-inline `INSERT INTO t VALUES (...)` path instead.

## Project Skills

| Skill | Description | Path |
|-------|-------------|------|
| grill-me | Interview the user relentlessly about a plan or design until reaching shared understanding, resolving each branch of the decision tree. Use when user wants to stress-test a plan, get grilled on their design, or mentions "grill me". | `.agents/skills/grill-me/SKILL.md` |
| skill-creator | Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Claude's capabilities with specialized knowledge, workflows, or tool integrations. | `.agents/skills/skill-creator/SKILL.md` |
| virgil | > Explore and query codebases using virgil-cli. Covers project registration, SQL templates for symbol search and call graph traversal, file reading. Use when asked to analyze a codebase, understand architecture, find symbols, trace callers/callees, onboard to a project, investigate bugs, or map the API surface of any TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebase. | `.agents/skills/virgil/SKILL.md` |

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
