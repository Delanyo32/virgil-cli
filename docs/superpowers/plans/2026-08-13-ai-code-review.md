# AI Code-Review CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform virgil-cli from a SQL-query CLI into an AI code-review tool: `virgil-cli scan <path>` parses a repo into DuckDB, then cersei-based agents (one per review prompt, running in parallel) query that database and report findings.

**Architecture:** The parsing engine (tree-sitter → DuckDB fact store) stays untouched except for one schema change: source text is stored in a new `file.content` column so agents never read the filesystem. Each review prompt (built-in or user markdown) becomes one cersei agent with three database-backed tools (`query`, `read_source`, `report_finding`). A tokio semaphore caps concurrent agents; each agent owns its own cloned DuckDB connection. Findings merge into one report (terminal / JSON / markdown).

**Tech Stack:** Rust 2024, duckdb 1.x (bundled), cersei 0.2.6 (+ async-trait, schemars), tokio (already present), clap 4.

**One addition beyond the grill decisions:** a third agent tool, `report_finding`. The grill locked two *reading* tools; findings still need a reliable structured channel back. Parsing findings out of the model's final prose is fragile; a tool call with typed fields is less code and can't mis-parse. Flagged here so the deviation is explicit.

## Global Constraints

- Branch: create `feat/ai-scan` from the current branch before Task 1.
- **Keep-list — do NOT delete these even though a dead-code audit flagged them** (each looks dead in the old tree but the new code needs it): `DbStore::try_clone_store` (the scan runner clones one connection per agent), `DbStore::run_script` (new tool tests seed data with it), the `extends`/`implements`/`throws`/`returns_type` tables and their resolution chain (the architecture/bugs review prompts query them), the FNV-1a hash in `cache_dir_for_db` (cache filenames must stay stable across Rust releases — do not swap for `DefaultHasher`).
- **PGQ is removed entirely in Task 3b** (only the deleted query templates ever used `GRAPH_TABLE`). After it lands there is no `duckpgq` install/load anywhere — cold builds stop downloading the extension, and `try_clone_store` no longer re-LOADs it.
- Run `cargo test` before every commit (standing rule). Commit messages: conventional style, **no Co-Authored-By lines**.
- `SCHEMA_VERSION` bumps to `6` exactly once (Task 1). Old caches self-wipe on open — that is the designed behavior, not a bug.
- cersei API shapes below were taken from its README; **Task 4 verifies them against the real crate first** and later tasks adapt names if they differ. Do not blindly trust the snippets in Tasks 5–6 if Task 4 found different names.
- The plan file itself lives in `docs/superpowers/plans/`. Task 3 deletes `docs/` **except** this directory; the final task may delete the plan too.
- Do not touch `.env` (root). Flag to the user at the end if it contains a real key.
- Keep: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `install.sh`, `RELEASING.md`, `LICENSE.md`, `SECURITY.md`, `CONTRIBUTING.md`, `tests/`.

## File Structure (end state)

```
src/
  cli.rs              # rewritten: Scan / InitPrompts / Clean
  main.rs             # rewritten: dispatch + tokio block_on for scan
  lib.rs              # drop serve/queries-template exports, add scan
  scan/
    mod.rs            # run_scan(): build/warm store, spawn agents, collect findings
    provider.rs       # ProviderKind → cersei provider construction
    tools.rs          # QueryTool, ReadSourceTool, ReportFindingTool
    prompts.rs        # built-in prompt embedding + --prompts loading + system prompt
    prompts/builtin/  # security.md, bugs.md, maintainability.md, architecture.md
    report.rs         # Finding, Severity, terminal/JSON/markdown rendering
  db/                 # unchanged except schema.rs, writer.rs, mod.rs (v6 + content)
  graph/              # unchanged except builder.rs (pass content through)
  languages/          # untouched
  storage/            # registry.rs DELETED; workspace/discovery/file_source stay
  queries/            # DELETED entirely (templates, runner, rust_templates)
  serve/              # DELETED entirely
  observability/      # mod.rs stays (log init); sampler.rs DELETED
```

---

### Task 1: Schema v6 — store source text in `file.content`

**Files:**
- Modify: `src/db/mod.rs:29` (`SCHEMA_VERSION` 5 → 6)
- Modify: `src/db/schema.rs:15-19` (file table DDL)
- Modify: `src/db/writer.rs:105` (`push_file` signature), `src/db/writer.rs:601` (flush arity)
- Modify: `src/graph/builder.rs:495` (call site — the file's source text is already in scope there; it was just parsed)
- Test: `tests/integration_test.rs`

**Interfaces:**
- Consumes: existing `DbWriter::push_file(&mut self, path, language, repo_id)`.
- Produces: `DbWriter::push_file(&mut self, path: &str, language: &str, repo_id: &str, content: &str)` and a `file.content VARCHAR NOT NULL` column that Tasks 5–6 read via `SELECT content FROM file WHERE path = ?`.

- [ ] **Step 1: Write the failing test**

Append to `tests/integration_test.rs` (reuse the existing fixture-building helpers in that file — it already builds a store from `tests/fixtures`):

```rust
#[test]
fn file_content_is_stored() {
    // Reuse whatever helper the existing tests use to cold-build a store
    // from tests/fixtures into an in-memory or temp DbStore.
    let (store, _ws) = build_fixture_store(); // adapt to the real helper name
    let rows = store
        .run_query(
            "SELECT content FROM file WHERE content <> '' LIMIT 1",
            Default::default(),
        )
        .unwrap();
    assert_eq!(rows.rows.len(), 1, "expected at least one file with stored content");
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo test file_content_is_stored`
Expected: FAIL — `Binder Error: column "content" does not exist` (or compile error if helper name differs; fix the helper name first, then confirm the Binder error).

- [ ] **Step 3: Implement**

`src/db/schema.rs` — file table gains one column:

```rust
"CREATE TABLE file (\
    path VARCHAR PRIMARY KEY, \
    language VARCHAR NOT NULL, \
    repo_id VARCHAR NOT NULL, \
    content VARCHAR NOT NULL\
 )",
```

`src/db/mod.rs`: `pub const SCHEMA_VERSION: u32 = 6;`

`src/db/writer.rs`: add the fourth field to `push_file` and its row tuple; bump the file-table column arity where it flushes (`flush_table(conn, "file", ...)` — match how the other 4-column tables pass arity).

`src/graph/builder.rs:495`: pass the already-in-scope source text:

```rust
stream_writer.push_file(&path, language_str, repo_id, &source);
```

(`source`/`content` — whatever the local holding the file text is named at that point; it must exist because the file was just parsed.)

- [ ] **Step 4: Run the full suite**

Run: `cargo test`
Expected: PASS, including the new test.

- [ ] **Step 5: Commit**

```bash
git add src/db tests/ src/graph/builder.rs
git commit -m "feat(db): store file content in fact store (schema v6)"
```

---

### Task 2: New CLI surface — `scan` / `init-prompts` / `clean`; delete `serve` and `projects`

The scan command in this task **only builds/warms the database and prints stats** — agents arrive in Task 6. That keeps this task compilable and testable on its own.

**Files:**
- Rewrite: `src/cli.rs`
- Rewrite: `src/main.rs`
- Modify: `src/lib.rs` (drop `serve`, `queries` modules; keep the rest)
- Delete: `src/serve/` (whole dir), `src/queries/` (whole dir), `src/storage/registry.rs`, `src/observability/sampler.rs`
- Modify: `src/storage/mod.rs` (drop `registry`), `src/observability/mod.rs` (drop `sampler`)
- Modify: `Cargo.toml` (remove `axum`, `async-stream`; keep `tokio` — the scan runner needs it)
- Test: `tests/integration_test.rs` (fix anything that imported deleted modules)

**Interfaces:**
- Consumes: `Workspace::load(root, &languages, None)`, `DbStore::open_persistent`, `db::cache_dir_for_db(id)`, `GraphBuilder::new(&ws, &langs).build(&store)`, `db::populate(&store, &graph, Some(&ws))` — all unchanged from `main.rs:134-174`.
- Produces: `cli::Command::{Scan, InitPrompts, Clean}` with the exact fields below; `scan::ScanConfig` consumed by Task 6; project identity = canonicalized path string passed to `cache_dir_for_db`.

- [ ] **Step 1: Rewrite `src/cli.rs`**

```rust
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "virgil-cli",
    about = "AI code review — parses your repo into DuckDB, then AI agents review it",
    version
)]
pub struct Cli {
    /// Increase log verbosity (-v info, -vv debug, -vvv trace). Overridden by VIRGIL_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress all logs except errors.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

// LogFormat is deliberately gone: the Json variant existed for serve-mode log
// shippers (observability/mod.rs's own comment says so). Compact-only now;
// observability::init loses its format parameter.

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Ollama,
    Openrouter,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Parse a repo and run an AI code review over it.
    Scan {
        /// Root directory of the project to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Markdown prompt file or directory (replaces the built-in reviews)
        #[arg(long)]
        prompts: Option<PathBuf>,

        /// Max review agents running at once
        #[arg(long, default_value_t = 4)]
        workers: usize,

        /// AI provider
        #[arg(long, value_enum, default_value_t = ProviderKind::Anthropic)]
        provider: ProviderKind,

        /// Model id (defaults to claude-opus-5 for anthropic; required otherwise)
        #[arg(long)]
        model: Option<String>,

        /// Emit findings as JSON instead of the terminal report
        #[arg(long)]
        json: bool,

        /// Also write a markdown report to this path
        #[arg(long)]
        output: Option<PathBuf>,

        /// Force a fresh rebuild of the cached fact store
        #[arg(long)]
        rebuild: bool,

        /// Glob patterns to exclude (repeatable)
        #[arg(short, long)]
        exclude: Vec<String>,

        /// Comma-separated language filter (ts,tsx,js,jsx,c,h,cpp,cs,rs,py,go,java,php)
        #[arg(short, long)]
        lang: Option<String>,
    },

    /// Copy the built-in review prompts into a directory so you can edit or extend them.
    InitPrompts {
        /// Destination directory (created if missing)
        dir: PathBuf,
    },

    /// Delete all cached databases (~/.cache/virgil).
    Clean,
}
```

Note: `exclude` is accepted but `Workspace::load` doesn't take excludes — the old flow filtered at registry time. Check `storage/discovery.rs`; if exclusion lived in `registry::create_project`, move that glob filtering into `Workspace::load`'s caller in `scan/mod.rs` (Task 6 wires it). If it's simpler, drop `--exclude` for now and note it in the README as future work — decide by reading `discovery.rs`, choose the smaller diff.

- [ ] **Step 2: Rewrite `src/main.rs`**

```rust
use anyhow::Result;
use clap::Parser;
use tracing::warn;

use virgil_cli::cli::{Cli, Command};
use virgil_cli::observability;

fn main() -> Result<()> {
    let cli = Cli::parse();
    observability::init(cli.verbose, cli.quiet);

    let result = dispatch(cli.command);
    if let Err(err) = &result {
        warn!(error = %err, "command failed");
    }
    result
}

fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Scan {
            path, prompts, workers, provider, model,
            json, output, rebuild, exclude, lang,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(virgil_cli::scan::run(virgil_cli::scan::ScanConfig {
                path, prompts, workers, provider, model,
                json, output, rebuild, exclude, lang,
            }))
        }
        Command::InitPrompts { dir } => virgil_cli::scan::prompts::init_prompts(&dir),
        Command::Clean => {
            let dir = dirs::cache_dir()
                .ok_or_else(|| anyhow::anyhow!("no cache dir"))?
                .join("virgil");
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
                println!("removed {}", dir.display());
            } else {
                println!("nothing to clean");
            }
            Ok(())
        }
    }
}
```

For this task only, create a stub `src/scan/mod.rs` so it compiles (Task 6 replaces `run`'s body):

```rust
pub mod prompts; // stub: only init_prompts for now — Task 5 fills this in
use crate::cli::ProviderKind;
use anyhow::Result;
use std::path::PathBuf;

pub struct ScanConfig {
    pub path: PathBuf,
    pub prompts: Option<PathBuf>,
    pub workers: usize,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub json: bool,
    pub output: Option<PathBuf>,
    pub rebuild: bool,
    pub exclude: Vec<String>,
    pub lang: Option<String>,
}

/// Builds (or warm-opens) the fact store for `cfg.path` and returns it
/// with the loaded workspace. Task 6 adds the agent phase after this.
pub async fn run(cfg: ScanConfig) -> Result<()> {
    let (_store, _ws) = build_store(&cfg)?;
    println!("store ready (agents arrive in a later task)");
    Ok(())
}

pub(crate) fn build_store(
    cfg: &ScanConfig,
) -> Result<(crate::db::DbStore, crate::storage::workspace::Workspace)> {
    use crate::language::{self, Language};
    let root = cfg.path.canonicalize()?;
    let languages = match &cfg.lang {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };
    let ws = crate::storage::workspace::Workspace::load(&root, &languages, None)?;

    let project_id = root.to_string_lossy().to_string();
    let cache_path = crate::db::cache_dir_for_db(&project_id)?;
    if cfg.rebuild && cache_path.exists() {
        std::fs::remove_file(&cache_path)?;
    }
    let store = crate::db::DbStore::open_persistent(&cache_path)?;
    if store.fresh() {
        let graph =
            crate::graph::builder::GraphBuilder::new(&ws, &languages).build(&store)?;
        crate::db::populate(&store, &graph, Some(&ws))?;
    }
    Ok((store, ws))
}
```

And a stub `src/scan/prompts.rs`:

```rust
use anyhow::Result;
use std::path::Path;

pub fn init_prompts(_dir: &Path) -> Result<()> {
    anyhow::bail!("init-prompts arrives in a later task")
}
```

- [ ] **Step 3: Delete the dead modules**

```bash
git rm -r src/serve src/queries
git rm src/storage/registry.rs src/observability/sampler.rs
```

Update `src/lib.rs`, `src/storage/mod.rs`, `src/observability/mod.rs` to drop the removed modules; add `pub mod scan;` to `lib.rs`. In `observability/mod.rs` also delete the `LogFormat` enum and the JSON branch — `init(verbose: u8, quiet: bool)` is the whole surface now (drop the `INITIALIZED` re-entry guard too; `init` has exactly one caller, the first line of `main`).

Remove from `Cargo.toml`: `axum`, `async-stream` (serve-only), `chrono` (its last user was `registry.rs`), `sysinfo` (last user was `sampler.rs`), and the `json` feature from `tracing-subscriber`. Chase compile errors — anything else that only served `serve`/`queries` (e.g. audit-shape formatting helpers) gets deleted too. **Rule: delete, don't comment out.**

- [ ] **Step 4: Fix tests, run everything**

`tests/integration_test.rs` likely calls `queries::run` for template tests — delete those test cases; keep parser/DB tests. Then:

Run: `cargo test && cargo build`
Expected: PASS. `cargo run -- scan tests/fixtures` prints "store ready…". `cargo run -- clean` works.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(cli)!: replace projects/serve surface with scan/init-prompts/clean"
```

---

### Task 3: Repo hygiene — delete dead directories and files

**Files:**
- Delete: `code-signatures/`, `.planning/`, `.scratch/`, `.virgil-skill/`, `.worktrees/`, `skill/`, `examples/`, `scripts/`, `Dockerfile.experiment`, `upload-to-s3.sh`, `.DS_Store`
- Delete: everything under `docs/` **except** `docs/superpowers/plans/` (this plan)
- Keep: `.env` untouched (flag to user at the end), release infra, `tests/`

- [ ] **Step 1: Delete**

```bash
git rm -r --ignore-unmatch code-signatures .planning .scratch .virgil-skill skill examples scripts
git rm --ignore-unmatch Dockerfile.experiment upload-to-s3.sh .DS_Store
find docs -mindepth 1 -maxdepth 1 ! -name superpowers -exec git rm -r {} +
find docs/superpowers -mindepth 1 -maxdepth 1 ! -name plans -exec git rm -r {} +
rmdir .worktrees 2>/dev/null || true
```

Also check `Cargo.toml` for `[[example]]`/`[[bench]]` sections referencing deleted files — remove them. Check `.github/workflows/ci.yml` for steps referencing deleted paths (e.g. examples) — trim those steps only.

- [ ] **Step 2: Verify the build is unaffected**

Run: `cargo test && cargo build`
Expected: PASS (these were non-code directories).

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "chore: delete pre-review-era directories, docs, and scripts"
```

---

### Task 3b: Dead-code purge — findings from the streamlining audit

Everything here was verified dead by a four-agent audit (zero surviving callers, checked by grep). Honor the **keep-list in Global Constraints** — `try_clone_store`, `run_script`, the inheritance tables, and the FNV hash all stay.

**Files:**
- Modify: `src/db/schema.rs`, `src/db/store.rs`, `src/db/writer.rs`, `src/db/from_code_graph.rs`, `src/db/mod.rs`
- Delete: `src/graph/intern.rs`
- Modify: `src/graph/mod.rs`, `src/graph/builder.rs`, `src/scan/mod.rs` (signature ripples)
- Modify: `src/models.rs`, `src/parser.rs`, `src/language.rs`, `src/storage/discovery.rs`, `src/languages/typescript/queries.rs` (+ every language module the `ImportInfo` field removal touches)
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: nothing new.
- Produces: `GraphBuilder::build(&self, store) -> Result<()>` (was `Result<CodeGraph>`), `db::populate(store: &DbStore) -> Result<()>` (was `(store, graph, workspace)`). Task 7's runner and Task 2's `build_store` stub must be updated to the new signatures in this task.

- [ ] **Step 1: Remove the PGQ chain**

Delete: `pgq_statements()` in `schema.rs` (~39 lines, the `CREATE PROPERTY GRAPH` DDL), the PGQ loop in `apply_schema`, `load_duckpgq` + `ensure_duckpgq_installed` in `store.rs` (including the call inside `try_clone_store` — the method itself stays), and the `pgq_match_walks_call_edges` test. Also delete the now-orphaned `all_builtin_templates_parse_against_empty_store` test (reads the deleted `builtin/*.sql` dir).

Grep-check afterward: `grep -rn "duckpgq\|GRAPH_TABLE\|PROPERTY GRAPH" src/ tests/` must return nothing.

- [ ] **Step 2: Remove the interner and `CodeGraph`**

Delete `src/graph/intern.rs` and the `CodeGraph` struct. In `builder.rs`, replace `file_known_spurs: HashSet<Spur>` with `HashSet<String>`, delete the tautological deferred-import guards at the old `builder.rs:312-317` (a `DeferredImport` only exists because the same call inserted the path), replace the `from_spur != to_spur` self-import check with `di.from_file_path != resolved`, and inline the `RESOLVE_IMPORTS_EAGERLY` const (it is `true`; the else-branch is unreachable). Change `GraphBuilder::build` to return `Result<()>` and `db::populate` to `populate(store: &DbStore)`. Update the callers in `src/scan/mod.rs` (`build_store`). Remove `lasso` from `Cargo.toml`.

- [ ] **Step 3: Purge dead db code**

Delete: `DbWriter::merge` (zero callers since the shared-writer refactor), `push_extends` / `push_implements` / `push_build_meta` + their buffers and flush lines (those tables are written by SQL/direct INSERT, not the writer), the `build_meta_files` table + `record_build_meta_files` + `push_build_meta_files` (no reader anywhere; `hash` is always `""`), the `idx_symbol_by_qname` and `idx_symbol_by_name_kind` indexes (zero SQL uses them). Drop the `vtab-arrow` and `appender-arrow` features from the `duckdb` dependency (no arrow API is used; removes 11 transitive crates).

**Do not bump `SCHEMA_VERSION` again** — it already went to 6 in Task 1, and no store built between Task 1 and here has shipped anywhere. Dropping `build_meta_files` and the indexes changes `create_statements()`, which is exactly what the version-6 wipe already covers. (If Task 1's commit was ever released separately, bump to 7 instead — not the case in this plan.)

- [ ] **Step 4: Purge dead API surface and dropped extractor output**

Delete `src/graph/metrics.rs` entirely (554 lines) plus `pub mod metrics;` and `GraphBuilder::find_node_at_line` (~27 lines) — their only caller was the deleted `complexity_hotspots` template. **Note: the compiler will NOT flag these** — `pub` items in a lib crate never trigger the dead-code lint, which is why this task lists them explicitly instead of relying on warnings.

Delete (all have zero non-test callers, verified): `discovery::discover_all_files` + its tests, `SymbolKind::from_str`, `parser::parse_content`, `Language::from_str`, `Language::extension` + its test, `models::ParseError`.

Remove the write-only fields `imported_name`, `local_name`, `is_type_only`, `line` from `models::ImportInfo` and let the compiler drive out the ~52 assignment sites across the language modules. In `typescript/queries.rs`, that deletes the entire import-binding machinery (`extract_import_bindings`, `extract_import_clause`, `extract_namespace_local`, `extract_named_imports`, `extract_import_specifier`, `extract_reexport_bindings`, `extract_export_specifier`, `has_type_keyword`, ~130 lines) — emit one `ImportInfo` per import statement instead of one per binding. Also remove the write-only `CommentInfo.associated_symbol_kind` field and its 9 assignment sites.

**Behavior check for the TS change:** before it, record `SELECT count(*), count(DISTINCT from_file || '>' || to_file) FROM imports` on a store built from `tests/fixtures`; after it, distinct edges must be unchanged (total rows may drop — those were per-binding duplicates).

- [ ] **Step 5: Verify**

Run: `cargo test && cargo build`
Expected: PASS. Then `cargo tree -d` to confirm no duplicate/orphan deps, and `grep -rn "lasso\|Spur\|CodeGraph" src/` returns nothing.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore: purge dead code — PGQ chain, interner, metrics, write-only fields"
```

---

### Task 4: Cersei spike — add the dependency and verify the real API

This task exists because the API shapes in Tasks 5–6 came from cersei's README via web fetch. **Verify before building on them.**

**Files:**
- Modify: `Cargo.toml`
- Create: `src/scan/provider.rs`
- Modify: `src/scan/mod.rs` (add `pub mod provider;`)

**Interfaces:**
- Produces: `provider::build(kind: ProviderKind, model: Option<&str>) -> Result<CerseiProvider>` where `CerseiProvider` is whatever concrete/boxed type `Agent::builder().provider(...)` accepts, plus `provider::default_model(kind) -> Option<&'static str>`. Task 6 consumes exactly these two functions.

- [ ] **Step 1: Add dependencies**

```bash
cargo add cersei async-trait schemars
```

If `cargo add cersei` pulls an enormous feature set, check `cargo info cersei` for feature flags and disable defaults down to: agent core + anthropic + openai providers. Keep the smallest set that compiles the spike below.

- [ ] **Step 2: Read the real API**

Run: `cargo doc -p cersei --no-deps --open` (or read `~/.cargo/registry/src/*/cersei-0.2.6/src/`). Confirm the actual names for:
1. Custom tool definition — README claims `#[derive(Tool)]` + `#[tool(name=…, description=…, permission=…)]` + `impl ToolExecute { type Input; async fn run(&self, input, ctx: &ToolContext) -> ToolResult }`, with `ToolResult::success(String)` / `ToolResult::error(…)`.
2. Builder — README claims `Agent::builder().provider(…).system_prompt(…).tool(T).model(…).max_tokens(…).build()?.run_with(prompt).await?` → `output.text()`.
3. Providers — `Anthropic::from_env()`, and `OpenAi::builder().base_url(…).api_key(…).model(…).build()`.
4. Whether the derive requires `schemars::JsonSchema` + `serde::Deserialize` on `Input`.
5. Whether an agent with **only** custom tools gets any built-in tools implicitly (it must not — if it does, find the builder switch that disables them).

**Write the confirmed names into this plan file** (edit Tasks 5–6 in place) if any differ.

- [ ] **Step 3: Write `src/scan/provider.rs` with the confirmed API**

```rust
// ponytail: provider construction only — no retry/config layers until needed
use crate::cli::ProviderKind;
use anyhow::{Context, Result};

pub fn default_model(kind: ProviderKind) -> Option<&'static str> {
    match kind {
        ProviderKind::Anthropic => Some("claude-opus-5"),
        _ => None, // openai/ollama/openrouter: user must pass --model
    }
}

/// Resolve the model id: --model wins, else the provider default, else error.
pub fn resolve_model(kind: ProviderKind, model: Option<&str>) -> Result<String> {
    model
        .map(str::to_string)
        .or_else(|| default_model(kind).map(str::to_string))
        .with_context(|| format!("--model is required for provider {kind:?}"))
}

// The return type here is whatever Agent::builder().provider() accepts —
// confirmed in Step 2. Sketch (adapt names to the real crate):
//
// pub fn build(kind: ProviderKind, model: &str) -> Result<impl cersei::Provider> {
//     match kind {
//         ProviderKind::Anthropic => Anthropic::from_env(),           // ANTHROPIC_API_KEY
//         ProviderKind::Openai    => OpenAi::from_env(),              // OPENAI_API_KEY
//         ProviderKind::Ollama    => OpenAi::builder()
//             .base_url("http://localhost:11434/v1")
//             .api_key("ollama")
//             .model(model)
//             .build(),
//         ProviderKind::Openrouter => OpenAi::builder()
//             .base_url("https://openrouter.ai/api/v1")
//             .api_key(std::env::var("OPENROUTER_API_KEY")?)
//             .model(model)
//             .build(),
//     }
// }
//
// If the four arms produce different concrete types that don't unify under
// `impl Trait`, return the crate's boxed/enum provider type — check what
// AgentBuilder::provider() takes and match it.
```

- [ ] **Step 4: Prove it compiles with a smoke test**

Add at the bottom of `provider.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ProviderKind;

    #[test]
    fn anthropic_default_model() {
        assert_eq!(
            resolve_model(ProviderKind::Anthropic, None).unwrap(),
            "claude-opus-5"
        );
    }

    #[test]
    fn openai_requires_model_flag() {
        assert!(resolve_model(ProviderKind::Openai, None).is_err());
    }

    #[test]
    fn ollama_provider_constructs_without_network() {
        // Construction must not hit the network — just build the value.
        let p = build(ProviderKind::Ollama, "qwen3");
        assert!(p.is_ok());
    }
}
```

Run: `cargo test scan::provider`
Expected: PASS. (If `Anthropic::from_env()` fails without a key, don't test that arm — construction requiring the env var is fine; the error message will guide users.)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/scan
git commit -m "feat(scan): add cersei with provider selection (anthropic/openai/ollama/openrouter)"
```

---

### Task 5: Agent tools — `query`, `read_source`, `report_finding`

**Files:**
- Create: `src/scan/tools.rs`
- Create: `src/scan/report.rs`
- Modify: `src/scan/mod.rs` (add `pub mod tools; pub mod report;`)
- Test: inline `#[cfg(test)]` in `tools.rs`

**Interfaces:**
- Consumes: `DbStore::run_query(&self, sql, BTreeMap<String, Value>) -> Result<QueryRows>` (`QueryRows { headers: Vec<String>, rows: Vec<Vec<Value>> }`), Task 1's `file.content` column, Task 4's confirmed cersei tool API.
- Produces:
  - `report::Severity` (`High | Medium | Low | Info`, ordered), `report::Finding { review: String, severity: Severity, file: String, line: Option<u32>, message: String }`.
  - `tools::make_tools(store: DbStore, findings: Arc<Mutex<Vec<Finding>>>) -> (QueryTool, ReadSourceTool, ReportFindingTool)` — Task 6 attaches these to an agent.

- [ ] **Step 1: Write `src/scan/report.rs` (data types only; rendering comes in Task 8)**

```rust
use cersei::prelude::schemars; // cersei re-exports schemars 0.8; see note below
use serde::{Deserialize, Serialize};

// `JsonSchema` is required because `ReportFindingInput` (Step 4) embeds
// `Severity`, and cersei's `ToolExecute::Input` bound is
// `DeserializeOwned + schemars::JsonSchema`. Do NOT `cargo add schemars` —
// `use cersei::prelude::*` re-exports cersei's own schemars 0.8, and a direct
// dependency on schemars 1.x would shadow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Which review reported it (filled by the runner, not the agent).
    #[serde(default)]
    pub review: String,
    pub severity: Severity,
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}
```

- [ ] **Step 2: Write the failing tool tests**

In `src/scan/tools.rs`, tests first (they define the contract; the impl follows):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_one_file() -> crate::db::DbStore {
        let store = crate::db::DbStore::open_in_memory().unwrap();
        store
            .run_script(
                "INSERT INTO file VALUES ('src/a.rs', 'rust', 'r', 'line one\nline two\nline three')",
                Default::default(),
            )
            .unwrap();
        store
    }

    #[test]
    fn query_rejects_writes() {
        let out = run_sql(&store_with_one_file(), "DELETE FROM file");
        assert!(out.starts_with("ERROR"), "writes must be rejected: {out}");
    }

    #[test]
    fn query_returns_json_rows() {
        let out = run_sql(&store_with_one_file(), "SELECT path FROM file");
        assert!(out.contains("src/a.rs"), "got: {out}");
    }

    #[test]
    fn read_source_slices_lines() {
        let store = store_with_one_file();
        let out = read_lines(&store, "src/a.rs", 2, 3).unwrap();
        assert_eq!(out, "line two\nline three");
    }

    #[test]
    fn read_source_unknown_file_is_error() {
        let store = store_with_one_file();
        assert!(read_lines(&store, "nope.rs", 1, 5).is_err());
    }
}
```

Run: `cargo test scan::tools` — Expected: compile FAIL (`run_sql` etc. undefined).

- [ ] **Step 3: Implement the tool core (plain functions first, cersei wrappers second)**

```rust
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::db::DbStore;
use crate::scan::report::Finding;

const MAX_ROWS: usize = 200;
const MAX_LINES: u32 = 400;

/// Read-only guard + run + serialize. Returned string goes straight to the model.
pub(crate) fn run_sql(store: &DbStore, sql: &str) -> String {
    let head = sql.trim_start().to_ascii_lowercase();
    // ponytail: prefix guard, not a SQL parser — trusted local CLI, the guard
    // only stops the model from accidental writes
    if !(head.starts_with("select") || head.starts_with("with")) {
        return "ERROR: only SELECT/WITH queries are allowed".into();
    }
    match store.run_query(sql, Default::default()) {
        Ok(rows) => {
            let total = rows.rows.len();
            let shown: Vec<_> = rows.rows.into_iter().take(MAX_ROWS).collect();
            let body = serde_json::json!({
                "headers": rows.headers,
                "rows": shown,
                "total_rows": total,
                "truncated": total > MAX_ROWS,
            });
            body.to_string()
        }
        Err(e) => format!("ERROR: {e}"),
    }
}

pub(crate) fn read_lines(store: &DbStore, path: &str, from: u32, to: u32) -> Result<String> {
    let rows = store.run_query(
        "SELECT content FROM file WHERE path = $path",
        std::collections::BTreeMap::from([("path".into(), path.into())]),
    )?;
    let row = rows.rows.first().with_context(|| format!("no such file in store: {path}"))?;
    // Value → String: adapt to how QueryRows encodes VARCHAR (check store.rs Value type).
    let content = value_as_str(&row[0])?;
    let from = from.max(1);
    let to = to.min(from + MAX_LINES - 1);
    let slice: Vec<&str> = content
        .lines()
        .skip(from as usize - 1)
        .take((to - from + 1) as usize)
        .collect();
    Ok(slice.join("\n"))
}
```

(`value_as_str`: small helper matching the actual `Value` enum in `store.rs` — likely `duckdb::types::Value::Text` or a local serde value; check and match.)

Note the `$path` binding: `run_query` supports `$name` substitution (the old query surface used it). If it only substitutes literals, that's fine here — path values come from the model but the connection is guarded read-only by the SQL prefix check, and this specific query is built by us with a bound param, not concatenation. Check how `run_script`/`run_query` bind and use the same mechanism.

- [ ] **Step 4: Wrap as cersei tools** (names per Task 4's verification)

Task 4 verified the real cersei 0.2.6 API. Bring the names into scope with
`use cersei::prelude::*;` — that glob supplies the `Tool` derive, `ToolExecute`,
`ToolContext`, `ToolResult`, the `#[async_trait]` attribute macro, and `schemars`.
Two extra direct dependencies are already in `Cargo.toml` and are **required**:
`cersei-tools` and `async-trait`, because the derive expands to
`cersei_tools::…` and `#[async_trait::async_trait]`, and neither of those crate
names is reachable through the prelude glob.

```rust
use cersei::prelude::*;

// Each agent gets its own DbStore clone; Mutex is uncontended, it exists
// only to satisfy Sync bounds on the tool struct. The `#[derive(Tool)]` /
// `#[tool(...)]` attributes below carry the name+description the model sees;
// `permission` accepts "none" | "read_only" | "write" | "execute" | "dangerous"
// and `category` is optional (defaults to Custom). `description` defaults to an
// empty string, so always set it.
#[derive(Tool)]
#[tool(
    name = "query",
    description = "Run a read-only SQL query against the code database. \
        Tables and columns are listed in the system prompt.",
    permission = "read_only"
)]
pub struct QueryTool(pub Arc<Mutex<DbStore>>);

#[derive(Tool)]
#[tool(name = "read_source", description = "...", permission = "read_only")]
pub struct ReadSourceTool(pub Arc<Mutex<DbStore>>);

#[derive(Tool)]
#[tool(name = "report_finding", description = "...", permission = "none")]
pub struct ReportFindingTool(pub Arc<Mutex<Vec<Finding>>>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct QueryInput {
    /// A read-only SQL query against the code database.
    pub sql: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadSourceInput {
    /// File path exactly as stored in the `file` table.
    pub path: String,
    /// First line, 1-indexed.
    pub start_line: u32,
    /// Last line, inclusive. At most 400 lines per call.
    pub end_line: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReportFindingInput {
    /// "high" | "medium" | "low" | "info"
    pub severity: crate::scan::report::Severity,
    pub file: String,
    pub line: Option<u32>,
    /// One clear sentence: what is wrong and why it matters.
    pub message: String,
}

// Confirmed cersei 0.2.6 shape: the derive sits on the struct (above), the
// behaviour goes in a separate `impl ToolExecute`.
#[async_trait]
impl ToolExecute for QueryTool {
    type Input = QueryInput;
    async fn run(&self, input: QueryInput, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success(run_sql(&self.0.lock().unwrap(), &input.sql))
    }
}
// ReadSourceTool → read_lines(...) mapped to ToolResult::success / ::error.
// ReportFindingTool → push a Finding (review name filled later by the runner),
//   reply ToolResult::success("recorded").
//
// The derive generates `Tool::input_schema` from
// `schemars::schema_for!(Self::Input)` (draft-07) and `Tool::execute`, which
// deserializes the JSON input and returns `ToolResult::error("Invalid input
// for '<name>': …")` on a bad payload — so tools never need to hand-validate.
//
// NOTE for the tests: `ToolContext` has no `Default` and no constructor, so
// unit tests should exercise `run_sql` / `read_lines` directly (as Step 2 does)
// rather than calling `Tool::execute`.

pub fn make_tools(
    store: DbStore,
    findings: Arc<Mutex<Vec<Finding>>>,
) -> (QueryTool, ReadSourceTool, ReportFindingTool) {
    let store = Arc::new(Mutex::new(store));
    (
        QueryTool(store.clone()),
        ReadSourceTool(store),
        ReportFindingTool(findings),
    )
}
```

- [ ] **Step 5: Run tests, commit**

Run: `cargo test scan::`
Expected: PASS.

```bash
git add src/scan
git commit -m "feat(scan): database-backed agent tools (query, read_source, report_finding)"
```

---

### Task 6: Review prompts — four built-ins, `--prompts` loading, system prompt, `init-prompts`

**Files:**
- Create: `src/scan/prompts/builtin/security.md`, `bugs.md`, `maintainability.md`, `architecture.md`
- Rewrite: `src/scan/prompts.rs` (replace the Task 2 stub)
- Test: inline `#[cfg(test)]` in `prompts.rs`

**Interfaces:**
- Consumes: `schema::create_statements()` from `src/db/schema.rs` (make it `pub` if it isn't).
- Produces: `prompts::ReviewPrompt { name: String, body: String }`, `prompts::load(custom: Option<&Path>) -> Result<Vec<ReviewPrompt>>`, `prompts::system_prompt() -> String`, `prompts::init_prompts(dir: &Path) -> Result<()>`.

- [ ] **Step 1: Write the four built-in prompt files**

`src/scan/prompts/builtin/security.md`:

```markdown
Review this codebase for security problems. Focus on:

- Injection: SQL, shell, or path strings built by concatenating untrusted input.
- Secrets committed to code: API keys, tokens, passwords in source or config.
- Unsafe input handling: missing validation at boundaries (HTTP handlers, CLI args,
  file parsing), unchecked deserialization.
- Dangerous patterns: `eval`-style execution, disabled TLS verification, weak
  hashing for credentials, world-readable file permissions.

Use the call graph: for each risky function you find (exec, query, open, spawn),
query its callers to see whether untrusted data can reach it.

Report only what you can point to in the code. Every finding needs a file, a line,
and one sentence on the attack it enables. Severity: high = exploitable now,
medium = exploitable with preconditions, low = hardening gap.
```

`src/scan/prompts/builtin/bugs.md`:

```markdown
Review this codebase for logic bugs. Focus on:

- Null/None/undefined misuse: values used without the check their type demands.
- Error handling gaps: ignored return values, empty catch blocks, errors that are
  swallowed and leave the program in a half-done state.
- Off-by-one and boundary mistakes in loops, slices, and index arithmetic.
- Mismatched assumptions: a caller passing arguments a callee doesn't expect
  (use the call_edge table to cross-check call sites against definitions).
- Copy-paste slips: near-identical branches where one forgot an edit.

Read the actual source of every function you suspect before reporting. Severity:
high = wrong result or crash on a common path, medium = wrong result on an edge
path, low = latent hazard.
```

`src/scan/prompts/builtin/maintainability.md`:

```markdown
Review this codebase for maintainability problems. Focus on:

- Oversized functions: use the span table to find functions spanning hundreds of
  lines, then read them to judge whether they genuinely do too much.
- Deep nesting and complex conditionals that resist understanding.
- Duplication: near-identical functions or blocks that should share one home
  (compare symbols with similar names across files).
- Naming that misleads: names that say one thing while the body does another.
- Dead weight: exported symbols nothing imports (join symbol against imports).

Do not report style preferences. Report only things that would slow down or
mislead the next person editing the file. Severity: high = actively misleading,
medium = costly to work around, low = friction.
```

`src/scan/prompts/builtin/architecture.md`:

```markdown
Review this codebase's structure using the dependency graph. Focus on:

- Import cycles: files or modules that import each other, directly or through a
  chain (walk the imports table).
- Layering violations: low-level modules importing high-level ones (infer layers
  from directory structure and import direction).
- God files: files that a large share of the codebase imports AND that import a
  large share of the codebase — both hub and authority.
- Dead exports: exported symbols with no importers anywhere.
- Inheritance tangles: deep or wide extends/implements chains (extends and
  implements tables).

This review lives in the graph tables — query first, read source only to confirm.
Severity: high = cycle or violation that blocks safe refactoring, medium =
structure that will rot, low = tidiness.
```

- [ ] **Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_load_by_default() {
        let ps = load(None).unwrap();
        let names: Vec<_> = ps.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["architecture", "bugs", "maintainability", "security"]);
    }

    #[test]
    fn custom_dir_replaces_builtins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my-check.md"), "look for X").unwrap();
        let ps = load(Some(dir.path())).unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "my-check");
    }

    #[test]
    fn single_file_prompt_works() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("solo.md");
        std::fs::write(&f, "check only this").unwrap();
        let ps = load(Some(&f)).unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "solo");
    }

    #[test]
    fn system_prompt_contains_schema() {
        let sp = system_prompt();
        assert!(sp.contains("CREATE TABLE file"));
        assert!(sp.contains("read_source"));
    }

    #[test]
    fn init_prompts_writes_four_files() {
        let dir = tempfile::tempdir().unwrap();
        init_prompts(dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
    }
}
```

Run: `cargo test scan::prompts` — Expected: FAIL.

- [ ] **Step 3: Implement `prompts.rs`**

```rust
use anyhow::{Context, Result};
use std::path::Path;

pub struct ReviewPrompt {
    pub name: String,
    pub body: String,
}

const BUILTINS: [(&str, &str); 4] = [
    ("architecture", include_str!("prompts/builtin/architecture.md")),
    ("bugs", include_str!("prompts/builtin/bugs.md")),
    ("maintainability", include_str!("prompts/builtin/maintainability.md")),
    ("security", include_str!("prompts/builtin/security.md")),
];

/// Built-ins by default; a `--prompts` file or directory replaces them.
pub fn load(custom: Option<&Path>) -> Result<Vec<ReviewPrompt>> {
    let Some(path) = custom else {
        return Ok(BUILTINS
            .iter()
            .map(|(n, b)| ReviewPrompt { name: n.to_string(), body: b.to_string() })
            .collect());
    };
    let mut out = Vec::new();
    if path.is_file() {
        out.push(read_prompt(path)?);
    } else {
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .with_context(|| format!("cannot read prompts dir {}", path.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        entries.sort();
        for p in entries {
            out.push(read_prompt(&p)?);
        }
    }
    anyhow::ensure!(!out.is_empty(), "no .md prompt files found in {}", path.display());
    Ok(out)
}

fn read_prompt(path: &Path) -> Result<ReviewPrompt> {
    Ok(ReviewPrompt {
        name: path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
        body: std::fs::read_to_string(path)?,
    })
}

pub fn init_prompts(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, body) in BUILTINS {
        let dest = dir.join(format!("{name}.md"));
        anyhow::ensure!(!dest.exists(), "{} already exists", dest.display());
        std::fs::write(&dest, body)?;
        println!("wrote {}", dest.display());
    }
    Ok(())
}

/// Shared system prompt: role, tools, the real DDL, example queries.
pub fn system_prompt() -> String {
    let ddl = crate::db::schema::create_statements().join(";\n");
    format!(
        "You are a code-review agent. The codebase you are reviewing has been \
parsed into a DuckDB database. You cannot access the filesystem, the network, \
or run code — the database is your only window into the codebase, and it is \
complete: every file's full source text is in file.content.

TOOLS
- query: run read-only SQL (SELECT/WITH) against the schema below.
- read_source: fetch a file's source lines (path, start_line, end_line). \
Prefer this over selecting file.content directly — it returns only the lines \
you asked for.
- report_finding: record one finding (severity, file, line, message). Call it \
once per distinct issue, as you confirm each one. Findings are the ONLY \
output that counts; prose in your final answer is discarded.

SCHEMA
{ddl}

QUERY HINTS
- Files and sizes: SELECT path, length(content) AS bytes FROM file ORDER BY bytes DESC LIMIT 20
- Symbols in a file: SELECT name, kind FROM symbol WHERE file_path = 'src/x.ts'
- Callers of a function: SELECT s.file_path, s.name FROM call_edge e \
JOIN symbol s ON s.id = e.caller_id JOIN symbol t ON t.id = e.callee_id \
WHERE t.name = 'target_fn'
- Who imports a file: SELECT from_file FROM imports WHERE to_file LIKE '%auth%'
- Never SELECT file.content in bulk; use read_source for code.

METHOD
Start broad (queries), narrow to suspects, confirm by reading source, then \
report. Verify line numbers against read_source output before reporting. \
When you have covered the review's focus areas, stop."
    )
}
```

Check the actual table/column names used in QUERY HINTS against `schema.rs` (e.g. `imports` columns, `call_edge` columns) and correct the hint SQL to match the real DDL — the DDL is embedded anyway, but the examples must not lie.

- [ ] **Step 4: Run tests, commit**

Run: `cargo test scan::prompts`
Expected: PASS. (`tempfile` is already a dev-dependency.)

```bash
git add src/scan
git commit -m "feat(scan): built-in review prompts, custom prompt loading, system prompt"
```

---

### Task 7: The scan runner — one agent per prompt, capped parallelism

**Files:**
- Rewrite: `src/scan/mod.rs` (`run` gets its real body)
- Test: inline `#[cfg(test)]` for the pure parts; end-to-end is manual (Task 9)

**Interfaces:**
- Consumes: `build_store` (Task 2, signatures as updated by Task 3b), `prompts::load`/`system_prompt` (Task 6), `tools::make_tools` (Task 5), `provider::build`/`resolve_model` (Task 4), `DbStore::try_clone_store()` (existing; after Task 3b it no longer loads any extension).
- Produces: `run(cfg: ScanConfig) -> Result<()>` printing via `report` (Task 8 fills rendering; until then print JSON).

- [ ] **Step 1: Implement the runner**

```rust
pub async fn run(cfg: ScanConfig) -> Result<()> {
    let (store, _ws) = build_store(&cfg)?;
    let prompts = prompts::load(cfg.prompts.as_deref())?;
    let model = provider::resolve_model(cfg.provider, cfg.model.as_deref())?;
    let system = prompts::system_prompt();

    let findings: Arc<Mutex<Vec<Finding>>> = Arc::default();
    let sem = Arc::new(tokio::sync::Semaphore::new(cfg.workers.max(1)));
    let mut handles = Vec::new();

    for prompt in prompts {
        // Clone BEFORE spawning: try_clone_store must run on a thread that
        // holds the original store.
        let agent_store = store.try_clone_store()?;
        let sink: Arc<Mutex<Vec<Finding>>> = Arc::default(); // per-agent, tagged after
        let all = findings.clone();
        let sem = sem.clone();
        let system = system.clone();
        let model = model.clone();
        let provider_kind = cfg.provider;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let name = prompt.name.clone();
            tracing::info!(review = %name, "review started");

            let (q, r, f) = tools::make_tools(agent_store, sink.clone());
            let provider = provider::build(provider_kind, &model)?;
            // Builder shape verified against cersei 0.2.6 in Task 4.
            // `provider_boxed` takes the `Box<dyn Provider>` that
            // `provider::build` returns. `run_with` lives on AgentBuilder and
            // consumes it — do NOT write `.build()?.run_with(..)`; that does
            // not compile. Use `.run_with(..)` alone, or `.build()?.run(..)`.
            // `max_turns` defaults to 10, `max_tokens` to 16384, and the
            // permission policy to AllowAll — set max_turns higher if reviews
            // need more SQL round-trips.
            let out = cersei::Agent::builder()
                .provider_boxed(provider)
                .system_prompt(&system)
                .model(&model)
                .max_turns(30)
                .tool(q)
                .tool(r)
                .tool(f)
                .run_with(&prompt.body)
                .await;

            match out {
                Ok(_) => {
                    let mut batch = sink.lock().unwrap();
                    for fnd in batch.iter_mut() {
                        fnd.review = name.clone();
                    }
                    let count = batch.len();
                    all.lock().unwrap().extend(batch.drain(..));
                    tracing::info!(review = %name, findings = count, "review finished");
                    Ok(())
                }
                Err(e) => {
                    // One failed review must not sink the scan.
                    tracing::warn!(review = %name, error = %e, "review failed");
                    Err(anyhow::anyhow!("review '{name}' failed: {e}"))
                }
            }
        }));
    }

    let mut failures = Vec::new();
    for h in handles {
        if let Err(e) = h.await? {
            failures.push(e.to_string());
        }
    }

    let mut findings = Arc::try_unwrap(findings)
        .expect("all agents done")
        .into_inner()
        .unwrap();
    findings.sort_by(|a, b| (a.severity, &a.review, &a.file).cmp(&(b.severity, &b.review, &b.file)));

    report::emit(&findings, &failures, cfg.json, cfg.output.as_deref())?;

    if findings.is_empty() && !failures.is_empty() {
        anyhow::bail!("all reviews failed");
    }
    Ok(())
}
```

Adaptation notes for the implementer:
- `.tool(t)` takes one `impl Tool + 'static` and can be chained, as above. `.tools(Vec<Box<dyn Tool>>)` also exists if a single call reads better.
- **No built-in tools are added implicitly.** `AgentBuilder::default()` starts with an empty `tools` vec, `build()` copies it verbatim, and the runner sends only `agent.tools` to the provider. An agent with three custom tools has exactly those three. There is no switch to turn off.
- The agent's own `.model(..)` is the only model that matters — the runner reads `agent.model` (falling back to `"claude-sonnet-4-6"`) and ignores the provider's `default_model`.
- The system prompt is passed through verbatim; the runner only appends a todo nudge when the `todo_write` tool has recorded todos, which never happens here.
- `run_with` returns `cersei_types::Result<AgentOutput>`; `CerseiError` is a `thiserror` enum, so `?` into `anyhow` works. `output.text()` returns `&str`.
- If `DbStore` is not `Send` (compiler will say so at `tokio::spawn`), fall back to `tokio::task::spawn_blocking` for the whole per-agent body with a small inner `Runtime::block_on` — but check first; `duckdb::Connection` is documented `Send`.
- `report::emit` doesn't exist yet — for THIS task stub it as `println!("{}", serde_json::to_string_pretty(findings)?)` inside `report.rs`; Task 8 replaces it.

- [ ] **Step 2: Compile-and-unit check**

Run: `cargo test && cargo build`
Expected: PASS (no live-API test here; end-to-end is Task 9).

- [ ] **Step 3: Commit**

```bash
git add src/scan
git commit -m "feat(scan): parallel per-prompt review agents with worker cap"
```

---

### Task 8: Report rendering — terminal, `--json`, `--output`

**Files:**
- Modify: `src/scan/report.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `Finding`, `Severity` (Task 5), sorted findings + failure list (Task 7).
- Produces: `report::emit(findings: &[Finding], failures: &[String], json: bool, output: Option<&Path>) -> Result<()>`.

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Finding> {
        vec![
            Finding { review: "security".into(), severity: Severity::High,
                      file: "src/auth.ts".into(), line: Some(42),
                      message: "SQL built by string concat".into() },
            Finding { review: "bugs".into(), severity: Severity::Medium,
                      file: "src/api.ts".into(), line: None,
                      message: "error swallowed".into() },
        ]
    }

    #[test]
    fn terminal_groups_by_review() {
        let s = render_terminal(&sample(), &[]);
        let sec = s.find("security (1 finding)").unwrap();
        let bug = s.find("bugs (1 finding)").unwrap();
        assert!(s.contains("HIGH"));
        assert!(s.contains("src/auth.ts:42"));
        assert!(sec < bug || bug < sec); // both present; order = severity of worst finding
    }

    #[test]
    fn markdown_has_table() {
        let s = render_markdown(&sample(), &[]);
        assert!(s.contains("| Severity |"));
        assert!(s.contains("src/auth.ts"));
    }

    #[test]
    fn failures_are_shown() {
        let s = render_terminal(&[], &["review 'bugs' failed: timeout".into()]);
        assert!(s.contains("failed"));
    }
}
```

- [ ] **Step 2: Implement**

```rust
use anyhow::Result;
use std::path::Path;

pub fn emit(
    findings: &[Finding],
    failures: &[String],
    json: bool,
    output: Option<&Path>,
) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "findings": findings,
                "failed_reviews": failures,
            }))?
        );
    } else {
        print!("{}", render_terminal(findings, failures));
    }
    if let Some(path) = output {
        std::fs::write(path, render_markdown(findings, failures))?;
        eprintln!("report written to {}", path.display());
    }
    Ok(())
}

fn sev_label(s: Severity) -> &'static str {
    match s {
        Severity::High => "HIGH",
        Severity::Medium => "MED",
        Severity::Low => "LOW",
        Severity::Info => "INFO",
    }
}

fn render_terminal(findings: &[Finding], failures: &[String]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    if findings.is_empty() {
        out.push_str("No findings.\n");
    }
    // Group by review, keeping input order (already severity-sorted).
    let mut reviews: Vec<&str> = Vec::new();
    for f in findings {
        if !reviews.contains(&f.review.as_str()) {
            reviews.push(&f.review);
        }
    }
    for review in reviews {
        let group: Vec<_> = findings.iter().filter(|f| f.review == review).collect();
        let noun = if group.len() == 1 { "finding" } else { "findings" };
        writeln!(out, "\n{review} ({} {noun})", group.len()).unwrap();
        for f in group {
            let loc = match f.line {
                Some(l) => format!("{}:{}", f.file, l),
                None => f.file.clone(),
            };
            writeln!(out, "  {:<5} {:<40} {}", sev_label(f.severity), loc, f.message).unwrap();
        }
    }
    for fail in failures {
        writeln!(out, "\nWARNING: {fail}").unwrap();
    }
    out
}

fn render_markdown(findings: &[Finding], failures: &[String]) -> String {
    use std::fmt::Write;
    let mut out = String::from("# Code review findings\n");
    writeln!(out, "\n{} findings\n", findings.len()).unwrap();
    writeln!(out, "| Severity | Review | Location | Finding |").unwrap();
    writeln!(out, "|---|---|---|---|").unwrap();
    for f in findings {
        let loc = match f.line {
            Some(l) => format!("`{}:{}`", f.file, l),
            None => format!("`{}`", f.file),
        };
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            sev_label(f.severity), f.review, loc,
            f.message.replace('|', "\\|")
        )
        .unwrap();
    }
    for fail in failures {
        writeln!(out, "\n> WARNING: {fail}").unwrap();
    }
    out
}
```

- [ ] **Step 3: Run tests, commit**

Run: `cargo test scan::report`
Expected: PASS.

```bash
git add src/scan
git commit -m "feat(scan): terminal, JSON, and markdown finding reports"
```

---

### Task 9: Docs rewrite, dead-code sweep, end-to-end verification

**Files:**
- Rewrite: `README.md`, `CLAUDE.md`
- Modify: anything the dead-code sweep flags

- [ ] **Step 1: Final dead-code sweep**

The big dead items were purged explicitly in Task 3b. This pass catches stragglers only:

Run: `cargo build 2>&1 | grep warning` and `cargo clippy --all-targets`; fix or delete what they flag. Then — because `pub` items in a lib crate never trigger the dead-code lint — grep-verify each remaining `pub fn` in `src/db/store.rs`, `src/storage/`, and `src/models.rs` has a caller outside its own tests; delete any that don't (honoring the Global Constraints keep-list). Finally grep each remaining `Cargo.toml` crate name under `src/` and drop unreferenced ones.

Run: `cargo test` — Expected: PASS.

Also note for the user (no action in this plan): with `duckpgq` gone, the glibc-only constraint documented in `.github/workflows/release.yml` may be liftable — static musl Linux builds become possible. Flag it; don't change release infra here.

- [ ] **Step 2: Rewrite `README.md`**

Content, in this order (write real prose, not this outline): what it is (one paragraph: AI code review that parses your repo into a queryable database, agents review via SQL not file-crawling); install (existing `install.sh` / `cargo install` path — keep whatever release.yml publishes); quickstart (`export ANTHROPIC_API_KEY=…`, `virgil-cli scan .`); flags table for `scan`; custom prompts (`init-prompts` → edit → `--prompts`); providers section (anthropic default, `--provider openai|ollama|openrouter` + which env var each needs, ollama example with a local model); how it works (3 sentences: tree-sitter parse → DuckDB fact store with source text → one agent per prompt with `query`/`read_source`/`report_finding` tools); supported languages list (from `language.rs`).

- [ ] **Step 3: Rewrite `CLAUDE.md`**

Keep: the "Working notes for Claude" lessons block (measure before theorizing, verify before claiming, etc. — those are timeless), threading constraints (tree-sitter `!Send` parser, `Arc` queries), duckdb/duckpgq gotchas, shared-writer builder description, persistence/warm-start section (updated: schema v6, content column, path-keyed identity). Replace: module layout (new `scan/` tree, deleted modules), build & run commands (scan/init-prompts/clean), delete everything about templates/serve/PGQ user surface/audit-shape convention. Add: cersei integration notes (tool trait shape as actually verified in Task 4, provider construction, one-agent-per-prompt runner, findings flow).

- [ ] **Step 4: End-to-end verification (manual — needs a model)**

```bash
cargo run --release -- scan tests/fixtures --workers 2
# or without an API key:
cargo run --release -- scan tests/fixtures --provider ollama --model qwen3:8b
```

Verify: agents run, at least the terminal report renders (empty findings on a tiny fixture is acceptable), `--json` emits valid JSON (`| jq .`), `--output /tmp/r.md` writes markdown, second run is warm (no re-parse), `--rebuild` re-parses. Then scan a real repo you have locally and read the findings for sanity.

- [ ] **Step 5: Final flags to the user**

Report at the end: (a) `.env` exists at repo root — confirm it holds no live key / is gitignored; (b) whether `--exclude` was wired or dropped (Task 2 decision); (c) any cersei API deviations found in Task 4.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "docs: rewrite README and CLAUDE.md for the AI-review CLI"
```

---

## Deferred follow-up (explicitly OUT of this plan's scope)

A streamlining audit found ~3,000 more lines of consolidation work in `src/languages/` and `src/db/writer.rs`. It is deferred to a follow-up branch because it is a wide refactor (40+ files) that would tangle the review of this transformation. Do **not** fold these into the tasks above. For the follow-up branch, ranked by payoff:

1. Nine copies of the comment→symbol association machinery (~680 lines) — pass the already-extracted `&[SymbolInfo]` into `extract_comments` instead of re-deriving names from the tree.
2. Nine copies of the references `Ctx` scaffolding (~595) — byte-identical except two one-line per-language skips; one shared `RefCtx` with a skip-list parameter.
3. `writer.rs` per-table boilerplate: 31 `push_*` fns + 42 flush calls → one `macro_rules!` (~480).
4. Nine byte-identical `extract_comments` (~350) and the nine-copy `extract_symbols` driver shell (~310).
5. 27 `compile_*_query` wrappers → one `query_sources(Language)` table (~170).
6. Nine copies of `find_node_at` → tree-sitter's own `Node::descendant_for_byte_range` (~135).
7. Double file read in the builder (parse reads, absorb re-reads for the generated-file marker) → compute `is_generated` at parse time; then the LRU cache in `DiskFileSource` never hits → delete it, the single-impl `FileSource` trait, and the `lru` crate (~85).
8. Merge the two SQL string-walk copies in `store.rs` (comment stripping + `$name` inlining) into one pass (~50); `pk_key` via `format!("{:?}", …)` (~30); small dup/stdlib items from the audit report (~450 across C/C++ shared fns, `symbol_id` ×10, `classify_comment` ×8, quote-strippers, `normalise_whitespace`).

Product calls left open: dropping `indicatif`/`tracing-indicatif` (one progress bar), `dirs` (one call), and whether all 10 language grammars stay.
