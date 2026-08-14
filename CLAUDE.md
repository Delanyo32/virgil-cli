# virgil-cli

Rust CLI that runs an AI code review over a repository. `virgil-cli scan <path>` parses
TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP with tree-sitter into a
file-backed DuckDB fact store in the OS cache dir (`<cache>/virgil/<hash>.duckdb`), then starts one
[cersei](https://crates.io/crates/cersei) agent per review prompt. Agents explore the
codebase through SQL and a source-slice tool; they never touch the filesystem or the
network. Findings come back as a grouped terminal report, JSON, or markdown.

## Working notes for Claude

Lessons from prior sessions on this codebase. Read this before doing real work.

**Measure before theorizing.** "Where is the memory going?" — don't guess. Add a checkpoint print and run. One memory-checkpoint pass beat hours of speculation about buffer pools, reduce intermediates, etc. Same rule for "is X slow?" / "does the channel block?" — measure, don't reason from first principles.

**Verify before claiming.** Bench-printing a matching scalar count is a sniff test, not verification. Before saying "this works" / "tests pass" / "matches baseline":
- Run `cargo test --release` and read the output. Don't skip it because the change "feels small."
- For data-shape claims, diff every relevant table count between branches. Don't extrapolate from one `COUNT(*)`.

**Don't pattern-match from training data on infrastructure facts.** "DuckDB needs single-threaded writes" (wrong — MVCC), "periodic flush will halve RSS" (wrong — saved 10%). Fetch the docs or read the source. Past sessions said these confidently and were wrong every time.

**Short answers for short questions.** "Why do we need X?" — two sentences. Not a table, not a "what this buys you" section. When the user types "simply put" or "caveman" or anything that signals brevity, that's a correction — match it on the next answer and stop reverting.

**Don't expand scope from a yes/no question.** "Can we do X?" → answer the question first, propose scope second, act after confirmation. "Let's bench this" is not authorisation for a multi-hour refactor.

**Surface regressions before merging.** A "fix" that makes the bench slower is a regression. Say so loudly, then let the user decide. Don't bury it under a tradeoff table.

**This codebase has specific known wrong claims to avoid repeating:**
- DuckDB supports concurrent readers via MVCC; `duckdb::Connection` is `Send` (not `Sync`) — use `try_clone`d siblings, not "one connection per process."
- The dominant memory term during parse+absorb is **per-worker scratch state in rayon's fold/reduce**, not `DbWriter` buffers and not DuckDB's buffer pool. The current shared-`Mutex<SharedAbsorb>` design avoids it.
- duckpgq / SQL/PGQ is **gone**. There is no property graph, no `GRAPH_TABLE`, and no extension download. Don't reintroduce PGQ syntax into queries or prompts.

## Build & Run

```bash
cargo build

# Review the current directory with the four built-in prompts.
cargo run -- scan .

# Narrower / cheaper runs.
cargo run -- scan ./src --workers 2 --lang rs
cargo run -- scan . --provider ollama --model qwen2.5:14b
cargo run -- scan . --json
cargo run -- scan . --output /tmp/review.md
cargo run -- scan . --rebuild          # wipe the cache and reparse

# Custom prompts.
cargo run -- init-prompts ./reviews
cargo run -- scan . --prompts ./reviews
cargo run -- scan . --prompts ./reviews/security.md

# Wipe every cached database.
cargo run -- clean
```

Three subcommands total: `scan`, `init-prompts`, `clean`. There is no `projects`
registry, no `serve` mode, no `--sql` / `--template` / `--file` query surface, and no
`--exclude` (discovery honours `.gitignore` instead). Global flags: `-v`/`-vv`/`-vvv`,
`--quiet`, and `VIRGIL_LOG` for a full `EnvFilter` string.

## Module Layout

- `src/cli.rs` — clap `Cli` / `Command` / `ProviderKind`. `ProviderKind` derives
  `ValueEnum`, so `to_possible_value()` is the source of truth for the `--provider`
  spelling
- `src/main.rs` — dispatch. `Scan` builds a tokio runtime and `block_on`s
  `scan::run`; `InitPrompts` and `Clean` are plain sync calls
- `src/scan/` — the review pipeline
  - `mod.rs` — `run(ScanConfig)`: `build_store` (parse or warm-open), load prompts,
    resolve the model, then one `tokio::spawn` per prompt gated by a `Semaphore`
    sized from `--workers`. `collect` / `merge` / `sort_findings` / `scan_failed` fold
    the per-review outcomes. `MAX_TURNS = 30`
  - `prompts.rs` — the four `include_str!`'d built-ins, `--prompts` loading (file or
    sorted `.md` directory), `init_prompts`, and `system_prompt()` — which embeds the
    live DDL from `db::schema::create_statements()` so the agent's schema can never
    drift from the real one
  - `provider.rs` — `resolve_model` + `build`. Anthropic and OpenAI come from
    `from_env`; ollama and openrouter are `OpenAi::builder()` with a `base_url`
  - `tools.rs` — the three `#[derive(Tool)]` types plus `run_sql` / `read_lines`
  - `report.rs` — `Severity`, `Finding`, and the terminal / JSON / markdown renderers
- `src/db/` — fact store
  - `schema.rs` — the 31 `CREATE TABLE` statements and 11 `CREATE INDEX` statements.
    No PGQ DDL
  - `store.rs` — `DbStore` over one `duckdb::Connection`. `lock_down` runs on every
    connection; `cache_dir_for_db` hashes the project id with FNV-1a
  - `writer.rs` — `DbWriter` batched row accumulator; on flush, one DuckDB `Appender`
    per non-empty table. The 9 `*_attrs` tables (`VARCHAR[]` columns) go through a
    batched literal `INSERT VALUES` path because the appender can't bind `Value::List`
  - `from_code_graph.rs` — `populate`: `resolve_inheritance` (SQL join of
    `raw_inheritance` ⨝ `symbol` ⨝ `imports`) and `resolve_and_emit_call_edges`
    (rayon-parallel, emits `call_edge`). Also owns `symbol_id`, `type_id`,
    `detect_todo_kind`, `is_doc_comment`, `is_generated_marker`
- `src/graph/` — build-time scratch state
  - `mod.rs` — just the `GraphNode` enum (`File` / `Package`) used by import
    resolution. The old `CodeGraph` and its interner are gone
  - `builder.rs` — `GraphBuilder::build`: rayon parse + shared-writer absorb
- `src/languages/` — one deep module per language plus a facade
  - `mod.rs` — language-agnostic `compile_*_query` / `extract_*` / `resolve_import`
  - `<lang>/{queries,types,attrs,references}.rs` — per-language tree-sitter queries
    and extractors
- `src/storage/` — `discovery.rs` (`ignore::WalkBuilder`), `workspace.rs`
  (`Workspace::load`, sizes + languages only), `file_source.rs` (`DiskFileSource`,
  reads on demand into a 256-entry LRU)
- `src/language.rs` — the `Language` enum: 12 variants, 19 extensions
- `src/classify.rs` — `is_test_file`, `is_barrel_file`
- `src/parser.rs` — `create_parser`, `parse_file`
- `src/observability/mod.rs` — tracing init. Compact format only; progress bars via
  `tracing-indicatif` when stderr is a TTY

## cersei integration

Verified against cersei 0.2.6 by building against it — not from memory.

**Agent construction.** `cersei::Agent::builder()` then `.provider_boxed(Box<dyn
Provider>)`, `.system_prompt(&str)`, `.model(&str)`, `.max_turns(u32)`, `.tool(t)` per
tool, and finally `.run_with(&user_prompt).await`. The model id must go on the **agent**
builder — the runner reads it from there, never from the provider.

**Tools.** `#[derive(Tool)]` on a unit/tuple struct with a `#[tool(name = …,
description = …, permission = …)]` attribute generates `name()` and the JSON input
schema; you write `#[async_trait] impl ToolExecute` with `type Input` and
`async fn run(&self, input, _ctx: &ToolContext) -> ToolResult`. `Input` must be
`DeserializeOwned + schemars::JsonSchema`. Return `ToolResult::success(String)` or
`ToolResult::error(String)`.
- `ToolExecute` requires `Sync`, so `QueryTool` / `ReadSourceTool` hold
  `Arc<Mutex<DbStore>>`. The mutex exists for the bound, not for contention — each
  agent owns its own `try_clone_store()` sibling.
- `cersei-tools` and `async-trait` are **direct** dependencies even though no source
  line names `cersei_tools`: the derive expands to `cersei_tools::…` and
  `#[async_trait::async_trait]`, and neither is reachable through `cersei::prelude::*`.
  A dep-pruning pass will look at `cersei-tools` and see zero references. Leave it.
- schemars comes from `cersei::prelude::schemars` (0.8). Do **not** `cargo add
  schemars` — a second version breaks the derive.

**Providers.** `Anthropic::from_env()` and `OpenAi::from_env()` read `ANTHROPIC_API_KEY`
/ `OPENAI_API_KEY`. Ollama and OpenRouter reuse `OpenAi::builder()` with a `base_url`
override; that builder requires an `api_key` and a `model`, so the ollama arm passes a
dummy key. Construction is offline — no request fires until `run_with`.

**Runner shape.** One agent per prompt, all spawned up front, each waiting on
`Semaphore::acquire_owned` before it runs. `try_clone_store()` and `provider::build`
both happen on the parent thread before the spawn: the clone must be taken while
holding the original store, and building the provider early makes a missing API key
fail the scan once instead of once per review.

**Findings flow.** `report_finding` pushes into a per-agent `Arc<Mutex<Vec<Finding>>>`.
The runner drains that sink on **both** the ok and error paths, because an agent that
dies to a rate limit usually died after reporting real findings. It then stamps the
review name onto each finding (the agent never sets it) and sorts by
`(severity, review, file)` — `Severity` declares variants worst-first, so ascending is
already the right order.

`scan_failed` is `findings == 0 && failures > 0`: the scan exits non-zero when nothing
was reported **and at least one** review broke. It is *not* "every review broke" —
three clean reviews plus one rate-limited review on a genuinely clean repo exits 1, and
the bail message still says "all reviews failed", which is wrong in that case. Any
finding at all flips it back to exit 0.

**Cost of the dependency.** cersei pulls 255 transitive crates (522 packages in
`Cargo.lock` total). That was accepted deliberately; don't re-litigate it in a sweep.

## Non-obvious Implementation Notes

**tree-sitter 0.26 (do not downgrade)**
Pinned to 0.26 because cersei-tools requires it and `tree-sitter` sets
`links = "tree-sitter"`, so exactly one version may exist in the graph. The grammar
crates only depend on `tree-sitter-language 0.1`, so they are unaffected.
`QueryMatches` still uses `streaming_iterator::StreamingIterator`, not
`std::iter::Iterator` — iterate with `while let Some(m) = matches.next()`.

**Threading constraints**
- `tree_sitter::Parser` is `!Send` — create a fresh instance per rayon task (never
  share or pool)
- `tree_sitter::Query` objects are `Arc`-shareable — compile once per language, share
  across threads
- Parse + absorb run on rayon; the review agents run on tokio. They never overlap:
  `build_store` finishes before the first agent spawns

**DuckDB is cut off from the filesystem and the network**
`lock_down` runs `SET enable_external_access=false` on every connection this process
opens — `open_in_memory`, `open_persistent`, `try_reopen`, and `try_clone_store`. Without
it, `read_text('/etc/passwd')` and `glob('/**')` are ordinary table functions any SELECT
can call, and an `https://` path auto-installs `httpfs` and fetches it. Agents compose
SQL from repository text they do not control, so that is a real exfiltration path. The
switch is one-way — DuckDB refuses to re-enable it — and it is why no extension can be
loaded any more. If you ever need an extension, that decision has to be revisited
explicitly, not worked around.

The `SELECT`/`WITH` prefix check in `tools::run_sql` is the *write* guard only, and it
is a prefix check, not a SQL parser. Comments are stripped first so `-- note\nSELECT …`
is judged on its real first keyword. Read protection is `lock_down`'s job, because no
prefix check catches `SELECT * FROM read_text(...)`.

**Shared-writer parallel graph builder**
`GraphBuilder::build` runs rayon `par_iter().try_for_each(...)` over the file list. Each
worker parses a file lock-free (tree-sitter + extractors), then briefly takes a
`Mutex<SharedAbsorb>` to push rows into a single shared `DbWriter` plus the cross-file
deferred Vecs. Periodic flush (`STREAM_FLUSH_EVERY_N_FILES = 200`) caps writer memory.
The critical section is short — Vec appends and a few HashMap inserts — so contention
doesn't dominate wall time.

Per-file resolution happens during absorb (file-local lookups via a local `name_to_id`
map plus a per-file `type_id_by_display` map). Cross-file refs are either queued for a
post-absorb Rust loop (`DeferredImport`) or written to the `raw_inheritance` staging
table for SQL resolution. `call_site` rows are also written during absorb, with only a
file-local `caller_id`; turning them into `call_edge` is `populate`'s job.

Alternatives explored earlier: (1) `mpsc::sync_channel` + single drainer — 25.7 s wall,
860 MiB RSS; (2) per-worker `WorkerLocal` with rayon `fold/reduce` — 16.5 s but 1.8 GiB;
(3) shared-writer (current) — 28.8 s, 760 MiB. Design 3 won because the memory
regression in 2 was structural to fold/reduce.

**Persistence + warm-start (schema v6)**
`build_store` canonicalizes `--path`, uses that absolute path string as the project id,
and derives `<dirs::cache_dir()>/virgil/<fnv1a-hash>.duckdb` from it via
`cache_dir_for_db` — that is `~/.cache/virgil` on Linux but `~/Library/Caches/virgil`
on macOS, so don't hard-code `~/.cache` in docs or tests. Two
different checkouts of the same repo get different caches; the same directory always
gets the same one. On open: if the file exists and `build_meta.schema_version` matches
the compiled-in `SCHEMA_VERSION`, the store reopens warm and `fresh()` is false, so the
parse is skipped entirely; otherwise the file is removed and a fresh schema applied.
`--rebuild` deletes the file before opening. `clean` removes the whole directory.

Incremental refresh is intentionally not implemented — the cache is reused whole or
rebuilt whole.

**Schema-version bumps**
`SCHEMA_VERSION` in `src/db/mod.rs` lives next to a changelog of every bump. Bump it
whenever `schema::create_statements()` or `index_statements()` changes shape; the open
path detects the mismatch and wipes stale stores. Currently `6` — v6 added
`file.content` (full source text) so the store is self-contained and readers never
touch the filesystem. That column is what `read_source` slices, and it is why an
agent's view of a file is exactly what the parser saw.

**File extension mapping**
- `.h` maps to C (deliberate). C++ headers must use `.hpp` / `.hxx` / `.hh`
- PHP uses `LANGUAGE_PHP` (handles `<?php` tags), not `LANGUAGE_PHP_ONLY`

**Python parsing**
`decorated_definition` nodes: unwrap to the inner function/class and skip the bare
`function_definition` / `class_definition` when its parent is a `decorated_definition`.
This prevents double-reporting decorated symbols.

**Call graph**
Name-based resolution scoped to the caller's imports. Heuristic only, no type info.
`builder.rs` emits raw `call_site` rows during absorb;
`from_code_graph::resolve_and_emit_call_edges` then reads `call_site` + `symbol` +
`imports` into Rust hash maps and materialises `call_edge`, so agent SQL joins one
table instead of redoing the resolution.

**`throws` extraction is not uniform across languages**
Java extracts the declared `throws` clause. C# and PHP have no declared throws keyword —
`extract_throws` walks `throw_statement` / `throw_expression` and pulls the type out of
`throw new X(...)` forms only; re-throws (`throw e;`) have no static type and emit no
row. The other languages return an empty `Vec<ThrowsRow>`. `absorb_file_data`
synthesises a `type{kind: "named"}` row inline when an exception type wasn't already
seen by `extract_types` in the same file, so the 3-way join through `type` succeeds.

**Python class-body assignments emit `Field` symbols**
`class C: x: int = 5` (and untyped `x = 5`) produce a `kind=field` `Symbol` row on top of
whatever the type extractor emits. That is what makes `symbol{kind: "field"} JOIN
field_type` non-empty.

**duckdb-rs gotchas (1.2)**
- `column_count` / `column_name` panic on a prepared-but-not-yet-queried statement — the
  schema isn't bound until execution. `run_query` snapshots headers from the first row's
  `as_ref()` after `query()` materialises the result set.
- `Appender::append_row` doesn't handle `Value::List` — `ValueRef::from(Value::List(..))`
  is `unimplemented!()`. The 9 `*_attrs` tables route through a batched literal-inline
  `INSERT INTO t VALUES (...)` path instead.
- `run_script` / `run_query` substitute `$name` placeholders as quoted SQL literals
  rather than binding them. Agent SQL passes an empty param map, so nothing is
  substituted on that path.

## Known limitations

Real, measured, and deliberately unfixed. Don't "discover" these again — and don't
quietly fix them inside an unrelated change.

- **`imports` resolves 0 rows on Rust corpora.** Import resolution works on TypeScript
  but produces nothing on Rust. Any review that leans on `imports` (the architecture
  prompt does) degrades to nothing on a Rust repo.
- **`extends` was empty on both tested corpora.** `resolve_inheritance` INNER JOINs to
  `symbol` for both endpoints, so a parent outside the workspace is dropped by design —
  but empty on every corpus tested so far points at something upstream too.
- **`from_code_graph.rs:74` has a dead join branch.** The `LEFT JOIN imports i … AND
  i.imported_id = parent.id` compares a file path (`imports.imported_id`) against a
  symbol id (`symbol.id`), so priority 2 ("imported") is unreachable and every
  cross-file parent falls to priority 3. Pre-existing; fixing it changes resolution
  cardinality, so it needs its own change with its own table-count diff.
- **`MAX_TURNS = 30` is a guess.** cersei's default of 10 is too low for
  query → read → report, but 30 was never tuned against a real scan. An agent that hits
  the cap loses whatever it hadn't reported yet.
- **A review can hang forever, and nothing breaks it.** Measured on 2026-08-13 against
  `--provider ollama` (`qwen2.5:7b` and `qwen2.5:14b`) on `tests/fixtures`: 4 of 5 live
  runs stopped making progress after the provider returned a normal `200`. `sample` on
  the stuck process shows all 36 threads parked (26 `__psynch_cvwait`, 13
  `semaphore_wait`, 2 `kevent` — the idle tokio driver), 0% CPU, and **zero open TCP
  sockets**, so the agent task is not waiting on the network; it is simply never woken
  again. It reproduces at `--workers 1`, so it is not contention between agents. There
  is no per-request timeout, no per-review timeout, and no scan deadline, so the CLI
  waits forever and prints nothing. Untriaged: it may be cersei 0.2.6's
  OpenAI-compatible client. Whoever picks this up should add a `tokio::time::timeout`
  around `run_with` first — that converts a hang into a failed review, which the runner
  already handles (findings survive, other reviews still report).
- **The `SELECT`/`WITH` prefix guard is not a parser.** See above — it stops writes, and
  `lock_down` is what stops reads. Don't "improve" one while forgetting the other.
- **Release builds are glibc-only.** `.github/workflows/release.yml` documents this as a
  duckpgq constraint: a static musl binary has no dynamic linker, so the extension
  `LOAD` failed. duckpgq is gone and nothing loads an extension any more, so the
  constraint is probably liftable and static musl Linux builds may now be possible.
  Untested — release infra was out of scope for the change that removed duckpgq.
