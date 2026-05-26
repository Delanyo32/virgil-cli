# Experiment: replace CozoDB backend with DuckDB + duckpgq

Branch: `worktree-experiment-duckdb` (worktree of `master`).

Goal: build a parallel implementation that swaps the Cozo SQLite-backed fact
store for a DuckDB store (with duckpgq for graph queries), benchmark both
against the existing 6-repo matrix, and decide a winner on speed, memory,
scale, and multithreaded CPU efficiency.

This plan was settled via `/grill-me` on 2026-05-25. The decisions below
are locked unless a finding during implementation forces a revisit.

---

## Locked decisions

| # | Topic | Decision |
|---|-------|----------|
| 1 | Branch structure | Full swap on `experiment/duckdb` worktree, no `Backend` trait abstraction. |
| 2 | Query language | PGQ (`GRAPH_TABLE(... MATCH ...)`) for graph templates; plain SQL elsewhere. |
| 3 | Schema | 1:1 port of all 28 Cozo relations; same column names + stringly composite IDs (ADR-0002). |
| 4 | Ingestion | Arrow `RecordBatch` builders per table, flushed via `INSERT INTO t SELECT * FROM arrow_scan(batch)`. |
| 5 | Query surface | `--cozoscript` flag removed; replaced by `--sql`. `$name` placeholders. `.cozoql` → `.sql`. |
| 6 | Persistence | Cold build + warm reopen only. Skip incremental refresh. |
| 7 | Bench | All 7 templates with realistic params; 3 runs per cell, take median. |
| 8 | PGQ DDL | Defined in `src/db/schema.rs` alongside table DDL. 2 vertex tables (`file`, `symbol`), 4 edge tables (`call_edge`, `imports`, `extends`, `implements`). |
| 9 | Scope | Drop `--s3` flag, `MemoryFileSource`, S3 fetcher, HTTP `serve` subcommand entirely. Local CLI only. |
| 10 | duckpgq install | `INSTALL duckpgq FROM community; LOAD duckpgq` on first cold-build. Cached in `~/.duckdb/extensions/`. |
| 11 | Module naming | `src/cozo/` → `src/db/`; `CozoStore` → `DbStore`, `CozoWriter` → `DbWriter`. Engine-neutral. |

## Deviations discovered during implementation

Recorded as we hit them — these aren't reversals of the locked plan, just
the actual SQL/PGQ shapes that worked once the parsers got involved.

1. **duckpgq vertex tables do not accept `KEY (col)` clauses.** The PK is
   taken implicitly from the table's primary key. `CREATE PROPERTY GRAPH
   codegraph VERTEX TABLES (file, symbol) ...` works; `file KEY (path)`
   fails to parse. Edge tables still need explicit `SOURCE KEY ...
   DESTINATION KEY ...`.

2. **`->*` and `->+` traversals require an explicit path mode.** WALK
   mode (the default) is rejected on unbounded traversals because
   cycles would produce infinite results. Use `MATCH ANY ACYCLIC (a)-[e:calls]->+(c)`
   for "any path." `find_callers` / `find_callees` are single-hop so
   don't need this, but anything transitive does.

3. **`WITH RECURSIVE` works; `WITH ... GRAPH_TABLE(...)` crashes.**
   Wrapping a `GRAPH_TABLE` clause in any kind of CTE triggers
   `INTERNAL Error: Attempted to dereference unique_ptr that is NULL!`
   on duckpgq 1.x. Consequence: `find_cycles` uses a recursive CTE
   over the materialised `*call_edge` table instead of PGQ. Pragmatic —
   recursive transitive closure + self-intersect for cycle detection
   is the natural SQL shape anyway.

4. **`$name` placeholders inside `GRAPH_TABLE(... WHERE ...)` are not
   bound by DuckDB's prepared-statement layer.** duckpgq parses the
   WHERE clause itself and the outer parameter binder never sees the
   `?` (or whatever you substitute). We switched from positional
   binding to literal SQL substitution (`inline_named_params` in
   `src/db/store.rs`): `$name` becomes a quoted literal at query
   construction time. Safe because `--param k=v` comes from trusted
   CLI input on the user's own machine.

5. **`--` line comments + `/* */` block comments are stripped before
   parsing.** Plain DuckDB accepts leading `--`; duckpgq's parser
   rejects them on the PGQ-flavored templates. We strip both kinds of
   comments programmatically before binding (`strip_sql_comments`).
   Side benefit: comments containing `$name` documentation references
   stop being treated as bindable placeholders.

6. **Appender doesn't support `Value::List` (VARCHAR[] columns).** duckdb
   1.2's `ValueRef::from(Value::List(..))` is `unimplemented!()`. The 9
   per-language `*_attrs` tables have `VARCHAR[]` columns, so they
   can't go through the Appender path. We flush them via a batched
   `INSERT INTO t VALUES (...), (...)` with values rendered as literal
   SQL (`flush_table_with_arrays` in `src/db/writer.rs`). Slower per
   row than Appender, but attrs rows are sparse so total cost stays
   small. If a future duckdb-rs version implements List binding, swap
   these tables back to the Appender path.

7. **`column_count` / `column_name` panic before `query()` is called.**
   duckdb 1.2's prepared-statement schema is bound lazily at execute
   time, not at prepare time. We snapshot column names from the first
   row's `as_ref()` statement after `query()` materialises the result
   set.

## Findings during end-to-end smoke (Phase 11)

- The pipeline works end-to-end on a tiny Rust workspace: cold build
  parses, populates DuckDB, runs all 7 templates + the
  `complexity_hotspots` Rust handler, returns valid output.
- **Pre-existing bug surfaced**: `call_site.caller_id` resolves to the
  nearest parameter rather than the enclosing function when the
  enclosing function has parameters. Example: `pub fn login(user: &str)
  { check_password(user) }` — the call to `check_password` records
  `caller_id` = `lib.rs|1|13|user|parameter` instead of
  `lib.rs|1|0|login|function`. Both backends see the same data
  (extractor lives in `src/graph/builder.rs`, unchanged by the swap),
  so this doesn't compromise the bench but should be filed against
  master separately.
- `find_callers` / `find_callees` outputs are therefore "wrong but
  consistent across backends" — perf comparison stays meaningful.

## Out of scope

- S3 support (`--s3`, `MemoryFileSource`, S3 fetcher)
- HTTP `serve` subcommand
- Incremental refresh / `workspace_diff`
- `cfg_languages/` and taint infrastructure (already deferred upstream)
- Surrogate-key schema modernisation (kept as Plan B if PGQ stringly joins prove catastrophic on `openclaw`)

---

## Architecture

### File store

- One DuckDB file per project at `~/.cache/virgil/<hash>.duckdb`.
- `cache_dir_for(id)` unchanged (still FNV-1a hash of project name).
- `DbStore::open_persistent(path)` opens the file; if the file exists and
  `build_meta.schema_version` matches the compiled-in `SCHEMA_VERSION` (and
  `build_meta.duckdb_storage_version` matches the linked DuckDB version),
  reopen warm. Otherwise wipe and rebuild.
- Connection opened writable so `INSTALL` of the duckpgq extension succeeds
  on first cold-build.

### Schema (`src/db/schema.rs`)

- `create_statements()` returns the `CREATE TABLE` DDL for all 28 relations
  (1:1 with current `:create` statements). Column types map:
  - Cozo `String` → DuckDB `VARCHAR`
  - Cozo `Int` → DuckDB `BIGINT`
  - Cozo `Bool` → DuckDB `BOOLEAN`
  - Cozo `String?` / `Int?` → nullable DuckDB column
  - Cozo `[String]` → DuckDB `VARCHAR[]`
  - Composite PKs translate to `PRIMARY KEY (col1, col2, ...)`.
- `index_statements()` returns `CREATE INDEX` statements mirroring the
  existing 13 indices.
- `pgq_statements()` returns one `CREATE PROPERTY GRAPH codegraph` DDL,
  applied after tables exist:
  - vertex tables: `file KEY (path)`, `symbol KEY (id)`
  - edge tables: `call_edge`, `imports`, `extends`, `implements`
- `SCHEMA_VERSION = 1` (fresh — no continuity with Cozo's v9).

### Writer (`src/db/writer.rs`)

- `DbWriter` owns one DuckDB writer connection.
- Per-table column builders (Arrow array builders from the `arrow` crate)
  accumulated in a struct.
- `flush()` finalises each per-table `RecordBatch`, registers it via the
  duckdb crate's Arrow integration, and runs `INSERT INTO <t> SELECT * FROM <arrow scan>`.
- Flush triggers unchanged: `STREAM_FLUSH_EVERY_N_FILES` files or a row-count
  threshold.

### Builder (`src/db/from_code_graph.rs`)

- Mostly unchanged. Emits the same logical rows; row appenders point at the
  Arrow array builders instead of Cozo `:put` strings.
- `resolve_and_emit_call_edges` keeps its parallel Rust + rayon shape: reads
  resolved `*call_site` / `*symbol` / `*imports` rows back out of DuckDB into
  `HashMap`s, resolves call sites on worker threads, writes results through
  the single `DbWriter`.

### Query runner (`src/queries/runner.rs`)

- `QueryRequest` payloads: `Sql(String)`, `Template { name, params }`,
  `File(PathBuf)`. (Replaces the Cozoscript variants.)
- `--param k=v` → `BTreeMap<String, duckdb::types::Value>`; auto-coerce keeps
  the int/bool short-circuit, everything else binds as `VARCHAR`. Parameters
  bind to `$name` placeholders.
- Audit-shape detection unchanged: column tuple
  `(file, line, severity, pattern, message)` → audit formatter.
- DuckDB connection is opened per query (existing CozoStore reused across
  queries — we keep that pattern).

### Templates (`src/queries/builtin/`)

- 7 files, all `.sql`:
  - `find_function_by_name.sql` — plain SQL
  - `find_implementations_of.sql` — plain SQL (one-hop join through `extends`/`implements`)
  - `export_surface.sql` — plain SQL
  - `find_callers.sql` — PGQ `GRAPH_TABLE(codegraph MATCH (a:symbol)-[:call_edge*]->(b:symbol) WHERE ...)`
  - `find_callees.sql` — PGQ (inverse direction)
  - `find_cycles.sql` — PGQ (cycle pattern)
  - `import_depth.sql` — PGQ (transitive `imports`)
- `complexity_hotspots` stays in `src/queries/rust_templates.rs`; its
  embedded Cozoscript becomes embedded SQL.

### CLI surface

- `projects query` flag changes:
  - `--cozoscript <s>` → `--sql <s>`
  - `--file <p>` unchanged (now reads `.sql`)
  - `--template <name>` unchanged
  - `--param k=v` unchanged
  - `--rebuild` unchanged
  - `--s3 <uri>` removed
- `serve` subcommand removed.

### Cargo dependencies

- Remove: `cozo`.
- Add: `duckdb = { version = "X", features = ["bundled"] }`, `arrow` for the
  ingest path (the duckdb crate's Arrow integration uses the arrow crate).
- Remove (with S3 + serve): `aws-sdk-s3`, HTTP server crates, anything else
  only used by those paths. Audit during the drop step.

---

## Phased work

1. **Cargo deps swap** — remove `cozo`, add `duckdb` + `arrow`; remove S3 + serve deps. Make `cargo check` pass with stubs in `src/db/`.
2. **Module rename** — move `src/cozo/` → `src/db/`; rename types; update imports across the crate. Keep stubs so callers compile.
3. **Drop S3 + serve** — delete `MemoryFileSource`, S3 fetcher, HTTP server, `--s3` flag, `serve` cmd. Simplify `FileSource` if only `DiskFileSource` remains.
4. **Schema port** — translate all 28 `:create` statements + 13 indices to DuckDB DDL; add `CREATE PROPERTY GRAPH codegraph` DDL.
5. **Writer port** — `DbWriter` with per-table Arrow builders + `arrow_scan` insert path.
6. **Builder port** — `absorb_file_data`, `populate`, `resolve_and_emit_call_edges` emit rows through `DbWriter`. Verify on a tiny workspace.
7. **Query runner port** — `QueryRequest` variants, `$name` param binding, audit-shape detection unchanged.
8. **Template rewrite** — 7 templates from `.cozoql` to `.sql`/PGQ. Verify each produces same row set as the current branch on a small repo.
9. **`complexity_hotspots`** — port embedded queries to SQL.
10. **Bench harness** — extend `bench_matrix.sh` to (a) accept `engine` label, (b) wipe `.duckdb` cache, (c) run all 7 templates post-build.
11. **End-to-end smoke** — register a small project (ripgrep or tokio), run all templates, compare outputs against the Cozo branch.
12. **Run the bench** — execute the matrix on both branches, write findings into `docs/experiments/duckdb-swap-findings.md`.

## Decision criteria

After the bench produces medianed numbers across all six repos and seven templates, declare a winner on each axis:

- **Speed**: `wall_s` (cold build + warm query, separately).
- **Memory**: `max_rss_mb` (cold build peak) + on-disk file size.
- **Scale**: behaviour at the openclaw extensions size (~5.5k files).
- **MT CPU**: `user_s / wall_s` ratio.

If DuckDB wins on most axes, schedule the follow-up to land this branch on
master (this will include porting incremental refresh + S3 + serve).

If Cozo wins, the branch is discarded; findings get a brief writeup so the
investigation isn't repeated.

If results are mixed (e.g. DuckDB wins memory but loses MT CPU), schedule
the Plan B experiment with surrogate-key IDs before deciding.
