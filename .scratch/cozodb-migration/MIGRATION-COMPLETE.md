# cozodb-migration — done

Summary of the migration from the JSON pipeline DSL to Cozoscript.

## What shipped (8 issues)

| # | Title | Commit |
|---|---|---|
| 01 | Bounded parse-worker channel | `ca50aac` |
| 02 | Cozo dep + cross-function schema + write path | `680c315` |
| 04 | `file_classification` + `nolint` facts (narrowed) | `cbec14e` |
| 05 | Cozoscript query surface + 10 templates (7 pure + 3 Rust) | `704d75f` |
| 06 | Delete JSON pipeline + audit subcommand | `1b4c67a` |
| 07 | SQLite persistence + warm-start | `e100693` |
| 08 | Incremental refresh | `04f9083` |
| 09 | Drop petgraph + delete unused taint/resource/cfg | `bbf822d` |

## What got deprecated (3 issues)

| # | Title | Why |
|---|---|---|
| 03 | CFG fact builder | CFG is recomputable on demand; storing it duplicates source-of-truth |
| ~~04 metric facts~~ | (partial) | Metrics deprecated same reason as 03; `file_classification` + `nolint` kept |
| 10 | graph-algo built-ins + 7 new templates | Upstream `graph_builder 0.4.1` doesn't compile against rayon 1.11 |

## End-state CLI surface

```bash
virgil-cli projects create <name> --path <dir> [--lang ...] [--exclude ...]
virgil-cli projects list
virgil-cli projects delete <name>

# Query — exactly one of --cozoscript / --file / --template required
virgil-cli projects query <name> --template <built-in> [--param k=v ...] [--rebuild] [--pretty]
virgil-cli projects query <name> --cozoscript '<inline>'
virgil-cli projects query <name> --file <path.cozoql>

# S3 (no registration needed)
virgil-cli projects query --s3 s3://bucket/prefix --template <name>

# HTTP server (Virgil Live)
virgil-cli serve --s3 s3://bucket/prefix [--host 127.0.0.1] [--port 0]
# Routes: GET /health, POST /query {cozoscript|template, params}
```

**Removed:**
- `virgil-cli audit ...` subcommand
- `--q '{json}'` JSON DSL on `projects query`
- `--no-cfg` / `--no-resource-graph` / `--symbols-only` flags
- `/audit/summary` and `/audit/{category}` HTTP routes

## Built-in templates (10)

**Pure Cozoscript (7):**
`find_callers`, `find_callees`, `find_cycles`, `find_function_by_name`,
`export_surface`, `import_depth`, `unused_symbols`.

**Rust-side handler (1):**
`complexity_hotspots` — needs source-level metric computation.

(The `taint_paths` and `unreleased_resources` stubs from issue 05 were
removed in issue 09 alongside the unused taint/resource modules.)

## Performance

Reference workspace: virgil-cli's own `src/` (~50 Rust files).

| State | Time | Notes |
|---|---|---|
| Cold | ~850 ms | Full parse + populate |
| Warm | ~17 ms | SQLite reopen + Cozoscript query |
| Incremental (1 file added) | ~410 ms | Re-parse one file + re-resolve edges |
| Incremental (1 file removed) | ~25 ms | Delete facts + re-resolve edges |

## LOC delta

Approximate, from `git diff c3e6e08..HEAD`:

- Deleted: ~22,000 lines (audit JSON files, pipeline DSL, query engine, taint/resource/cfg, 9 lang cfg builders)
- Added: ~3,500 lines (cozo module, queries module, templates, incremental refresh)
- Net: **~-18,000 LOC**

## Architecture changes

**Before:**
```
parse -> CodeGraph -> AuditEngine -> 297 JSON pipelines -> findings
parse -> CodeGraph -> JSON DSL query engine -> results
```

**After:**
```
parse -> CodeGraph -> populate -> Cozo store
                                      |
                                      v
        --template <name>  --->  runner  ---> rows or audit findings
        --cozoscript '...'       |
        --file <path>            +-> rust_templates for source-aware
                                     queries (complexity_hotspots)
```

CodeGraph is now backed by adjacency-list Vecs (not petgraph). It lives
only during build, gets walked once by `cozo::populate`, then dropped.
All query-time work hits the SQLite-backed Cozo store.

## Deps changed

**Added:**
- `cozo` (via `cozo-ce 0.7.13-alpha.3`), features `storage-sqlite` + `rayon`

**Removed:**
- `petgraph`

## Open follow-ups

- `taint_paths` and `unreleased_resources` are no longer reachable. If
  intra-function taint or resource lifecycle analysis is wanted back,
  it would be a new design (Cozo-backed, not the deleted petgraph
  walker).
- RocksDB backend remains unexplored (cozo-ce feature flag confusion +
  C++ dep cost). SQLite suffices for current scale.
- Multi-process safety under concurrent CLI + serve hasn't been stressed.
- No cache-size eviction; `~/.cache/virgil/` grows monotonically.
