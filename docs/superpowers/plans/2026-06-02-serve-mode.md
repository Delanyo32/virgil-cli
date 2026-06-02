# Serve mode

Date: 2026-06-02

Add a `serve` mode that exposes an **already-parsed** project over a local HTTP
API and answers queries — including the 10-minute-plus ones — concurrently,
without re-parsing per request.

The old `Serve` subcommand (dropped in the DuckDB swap, cli.rs:40) is not
reused; nothing from it survives. This is a fresh design.

## Goals

- Keep one project's warm DuckDB store resident and answer many queries against
  it without per-query parse/open cost.
- Survive 10-min-plus queries: a query must not be lost to client/proxy
  timeouts or a dropped connection.
- Run multiple queries genuinely in parallel (DuckDB MVCC, one connection per
  worker).

## Non-goals

- **Building/parsing is out of scope.** Serve only exposes a store that is
  already warm. Use whatever currently builds it (`projects query …`,
  `--rebuild`). No new `build` command.
- No multi-project routing, no project eviction (one project per server
  instance).
- No rebuild-while-serving / incremental refresh.
- No auth, no TLS, no remote access (localhost only).
- No daemonization, pidfile, or single-instance lock.

## Decisions (resolved during grilling)

| # | Decision | Choice |
|---|----------|--------|
| 1 | Transport | HTTP on `127.0.0.1` |
| 2 | Query execution model | Async **job** model: submit → `job_id` → result delivered later |
| 3 | Concurrency mechanism | Bounded worker concurrency, **one `try_clone`'d connection per in-flight job** |
| 4 | Concurrency cap `N` | **4**, configurable (`--max-concurrency`) |
| 5 | Project scope | **One project per server instance** |
| 6 | Startup | Expose **only an already-parsed (warm) store**; if `store.fresh()` → error and exit |
| 7 | HTTP library | **axum + tokio** |
| 8 | Cancellation/timeout | **Cooperative** — pending jobs cancel truly; running jobs can't be force-stopped (no DuckDB `interrupt`), only marked abandoned; timeout is advisory |
| 9 | Result delivery | **SSE** stream per job, plus `GET /jobs/{id}` snapshot fallback |
| 10 | Result retention | **TTL eviction** — finished jobs are dropped after `--result-ttl-secs` (default 600); queued/running jobs are never evicted. A background sweeper runs every `ttl/4` (clamped 5–60s) |
| 11 | Process model | **Foreground**, Ctrl-C → stop accepting, **exit immediately** (don't drain in-flight) |
| 12 | Query sources exposed | `sql` + `template` + `params`; **`file` dropped** (server-side path footgun) |
| 13 | Connection structure | **`try_clone` pool of `DbStore`s**; `queries::run` reused unchanged |

### Key verified facts (don't re-derive)

- `duckdb::Connection::try_clone()` exists — "Creates a new connection to the
  already-opened database." This is the supported way to get N sibling read
  connections sharing one DB. (lib.rs:558 in duckdb 1.2.2.)
- DuckDB engine is concurrent: multiple connections read in parallel (MVCC);
  per the upstream concurrency docs, this is the recommended model — not a
  workaround.
- duckdb-rs 1.2.2 exposes **no** `interrupt` / `pending` / `execute_tasks` /
  async API. `stmt.query()` is a synchronous call that pins its OS thread for
  the query's full duration. → Concurrency comes from N threads × N
  connections; an in-flight query **cannot** be force-cancelled.
- `tokio` does not change the above: a blocking DuckDB call must run on a
  blocking thread (`spawn_blocking`), never on an async task (it would starve
  the executor). Axum/tokio is used for the HTTP layer and for cheaply holding
  idle SSE/poll connections.

## Architecture

```
                 ┌────────────── tokio runtime (axum) ──────────────┐
  HTTP client →  │  POST /query   → enqueue Job, return job_id       │
                 │  GET  /jobs/{id}         (snapshot)               │
                 │  GET  /jobs/{id}/events  (SSE, awaits completion) │
                 │  DELETE /jobs/{id}       (cooperative cancel)     │
                 │  GET  /health                                     │
                 └───────────────────────┬───────────────────────────┘
                                          │ submit
                              ┌───────────▼────────────┐
                              │  JobRegistry (in-mem)   │  Mutex<HashMap<JobId, Job>>
                              │  job_id → status/result │  + tokio Notify per job
                              └───────────┬────────────┘
                                          │ spawn_blocking, gated by Semaphore(N=4)
                          ┌───────────────▼───────────────┐
                          │  ConnectionPool                │  N try_clone'd DbStores
                          │  check out 1 DbStore per job   │  (each Mutex uncontended)
                          └───────────────┬───────────────┘
                                          │ queries::run(QueryRequest{ store, workspace, … })
                                          ▼
                                   DuckDB (warm file)
```

Resident state held by the server for its lifetime:

- `Workspace` (loaded once; needed by source-reading templates like
  `complexity_hotspots`). Cheap — sizes + lazy LRU disk reads.
- `ConnectionPool` of `N` `try_clone`'d `DbStore`s over the project's warm
  `.duckdb` file.
- `JobRegistry` — in-memory map, never evicted (decision 10).
- `Semaphore(N)` capping concurrent executions.

## API

```
POST   /query
       body: { "sql": "...", "params": {...} }
          or { "template": "find_callers", "params": {...} }
       → 200 { "job_id": "..." }                       (returns immediately)

GET    /jobs/{id}
       → { "status": "...", "result": {...}? , "error": "..."? }   (snapshot)

GET    /jobs/{id}/events                                (SSE)
       event: status   data: {"status":"running"}
       event: completed data: { result envelope }       (then stream closes)
       event: error    data: {"error":"..."}

DELETE /jobs/{id}
       → 200 { "status": "cancelled" | "running" }       (A semantics)

GET    /health
       → { "project": "...", "ready": true, "schema_version": N }
```

- `status` ∈ `queued | running | done | error | cancelled | timed_out`.
- Result envelope mirrors today's CLI JSON: `{ project, query_ms, result }`
  where `result` is the existing `QueryOutput` (`Findings` | `Rows`) untouched.
- Client ergonomics: a thin client wraps `POST /query` + SSE into one awaitable
  (`const r = await runQuery(sql)`), so the frontend sees a single call that
  fires on completion — `job_id` and streaming stay hidden plumbing.

## Job lifecycle

1. `POST /query` → validate body, mint `JobId`, insert `queued` job, return id.
2. A tokio task acquires a `Semaphore` permit, then `spawn_blocking`:
   - check out a `DbStore` from the pool,
   - mark `running`,
   - call `queries::run(QueryRequest { source, params, store, workspace })`,
   - store `done`+result or `error`, return the `DbStore` to the pool, release
     permit, `Notify` waiters.
3. SSE handler awaits the job's `Notify`; emits `status` then `completed`/`error`.
4. `DELETE /jobs/{id}`:
   - `queued` → remove before execution = true cancel.
   - `running` → mark abandoned; result discarded on completion; **query keeps
     running** (no interrupt). Document this.
5. Advisory timeout (optional `timeout` in request or server default): on
   expiry mark `timed_out` and stop reporting; the thread still runs to
   completion. Off by default.

## Code structure / changes

- **`src/cli.rs`** — add `Serve { name, port (default 7777), max_concurrency
  (default 4) }` under `Command` (sibling of `Projects`). Bind `127.0.0.1`.
- **`src/db/store.rs`** — add `DbStore::try_clone_store(&self) -> Result<DbStore>`
  that `try_clone`s the inner `Connection` into a new `DbStore` (re-`LOAD
  duckpgq` on the clone; `fresh = false`). No change to `run_query`.
- **`src/serve/` (new module)**
  - `mod.rs` — `serve(name, port, max_concurrency)`: load project + `Workspace`,
    open persistent `DbStore`; **if `store.fresh()` → bail** with a message
    pointing at the existing build path; else build `ConnectionPool` and launch
    the tokio/axum server.
  - `pool.rs` — `ConnectionPool` over N `try_clone`'d `DbStore`s (e.g. an
    `ArrayQueue`/`Mutex<Vec<DbStore>>` checkout; permits from the shared
    `Semaphore`).
  - `jobs.rs` — `JobRegistry`, `Job`, `JobStatus`, per-job `Notify`.
  - `http.rs` — axum router + handlers (the 5 endpoints), SSE wiring.
- **`src/main.rs`** — dispatch `Command::Serve` → `serve::serve(...)`. Serve is
  long-running; reuse the existing `observability::init` for logging (per-job
  `tracing` span keyed by `job_id`).
- **`Cargo.toml`** — add `axum`, `tokio` (rt-multi-thread, macros), `tower`
  (as needed). `serde`/`serde_json` already present.

`queries::run`, `QueryRequest`, `QueryOutput`, templates, schema, and the whole
parse/build pipeline are **untouched**.

## Open implementation notes / risks

- `spawn_blocking` default pool is large (512); our `Semaphore(N)` is what
  actually bounds concurrency — don't rely on the blocking pool size.
- `DbStore::try_clone_store` must `LOAD duckpgq` on each clone (extension load
  is per-connection). `INSTALL` is already process-once-guarded.
- Pending-job queue is unbounded; pending jobs are cheap (just request specs),
  so no backpressure cap initially. Revisit if it matters.
- Large result sets are held in memory until restart (decision 10) and sent as
  one SSE `completed` event. Acceptable per the user; revisit if a single
  result OOMs.
- Exit-immediately on SIGINT abandons in-flight queries' threads; the process
  dies and the OS reaps them. No graceful drain.

## Verification

- `cargo test --release` green (existing suite unaffected; serve code is
  additive).
- New tests:
  - `try_clone_store` yields a working independent read connection (PGQ query
    succeeds on the clone).
  - `serve` against a `fresh()` store errors and exits non-zero.
  - Job lifecycle: submit → SSE `completed` carries the same envelope as the
    equivalent `projects query`.
  - `DELETE` a queued job cancels; `DELETE` a running job returns `running` and
    does not corrupt the registry.
- Manual: start `serve`, fire two concurrent long queries via curl, confirm
  both run in parallel (wall-clock ≈ max, not sum) and SSE delivers each.
