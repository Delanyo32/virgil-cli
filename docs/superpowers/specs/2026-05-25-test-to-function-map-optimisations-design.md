# Design — `test_to_function_map` query optimisations + bench harness

**Date:** 2026-05-25
**Branch:** `feat/test-to-function-map-optimisations` (off master)
**Scope:** prototype + bench. Not a production rollout.

## Problem

A single `virgil-cli projects query` invocation running the orchestrator's
`test_to_function_map.cozoql` pegs one core for several minutes on a 4 GB
SQLite store. The query rebuilds the entire call graph on every invocation,
then filters most of it away with a regex on file paths. Other audits in the
same orchestrator run pay the same per-process cost. CPU does not spread
because Cozo evaluates a single recursive rule on one core, and the
orchestrator runs audits one process at a time.

This spec covers a four-part change to `test_to_function_map` and a bench
harness to quantify the effect across repo sizes.

## Goals

- Make `test_to_function_map` finish faster on the same input.
- Measure the speedup honestly: wall-clock, CPU time, peak RSS, across
  multiple repo sizes, on cold runs.
- Keep the change small enough to revert if it doesn't pay off.

## Non-goals

- Other orchestrator queries. Scope is `test_to_function_map` only.
- Cross-process concurrency / `query-batch` subcommand. Separate prototype.
- Upstream Cozo patches (within-rule clause parallelism).
- Production rollout, docs, or migration plan for existing caches.

## Changes (4 parts)

### 1. Replace regex filter with relation join (change #2)

`regex_matches(file, "(?i)(test|spec|__tests__|\.test\.|\.spec\.)")` is
replaced by a join on `*file_classification{file_id: file, is_test: true}`.
The `is_test` fact is already populated at build time by
`src/classify.rs::is_test_file`.

### 2. Push the test-file filter before the call_edge join (change #3)

Currently `call_edge` is materialised over the entire workspace, then the
test-file filter prunes it down. New shape: filter `*call_site` to callers
whose file is a test file first, then resolve callees. Smaller working set
into the join.

### 3. Persist `call_edge` at build time (change #4a)

A new `*call_edge {caller_id, callee_id => file_path}` relation is added to
the schema. Populated at the tail of `cozo::populate(...)` by a new
`resolve_and_emit_call_edges(&store, &graph, &mut writer)`, using the same
two-rule name+kind resolution algorithm (intra-file first, then cross-file
via `*imports` + `exported=true`) the query was doing at query time.

After this change, queries that need resolved call edges read `*call_edge`
directly. Cost shifts from per-query to once-per-build.

Schema impact:
- `src/cozo/schema.rs` gains one `:create` statement for `call_edge`.
- `src/cozo/mod.rs` bumps `SCHEMA_VERSION: u32 = 8 → 9`. Existing caches
  are wiped on first open of the new binary (existing behaviour).

### 4. Add indices on hot join keys (change #5)

In `src/cozo/schema.rs::index_statements()`:
- `::index create call_site:by_caller {caller_id}`
- `::index create symbol:by_name_kind {name, kind}`
- `::index create imports:by_importer_imported {importer_file_id, imported_id}` — check whether `imports:by_importer` already covers this; if so, drop this one.

## Final query shape (after all four changes)

```cozoscript
?[file, line, severity, pattern, message] :=
    *call_edge{caller_id: c, callee_id: t, file_path: file},
    *file_classification{file_id: file, is_test: true},
    *symbol{id: c, name: caller_name},
    *symbol{id: t, name: callee_name},
    *span{entity_id: c, file_path: file, start_line: line},
    severity = "info",
    pattern = "test_call",
    message = concat("test=", caller_name, "|callee=", callee_name)
```

One rule, three indexed joins, no recursion, no regex.

## Architecture

### virgil-cli (feat branch)

```
src/cozo/
├── mod.rs                  # SCHEMA_VERSION: 8 → 9
├── schema.rs               # new *call_edge :create + 3 ::index lines
└── from_code_graph.rs      # new resolve_and_emit_call_edges() called from populate()
```

No new dependencies. No new MCP layer. ~150–200 LoC plus the rewritten
query file.

### orchestrator (virgil-audit repo, separate commit)

```
orchestrator/data/queries/test_to_function_map.cozoql   # rewritten body
```

Output columns unchanged. Downstream consumers untouched.

### Bench harness (feat branch only)

```
examples/bench_query_optimisations.sh    # bash, /usr/bin/time -lp parser
examples/bench_query_optimisations.md    # README
```

Bench script lives only on the feat branch (not committed to master).

## Data flow

### Build, optimised

1. `GraphBuilder::build(&store)` — unchanged. Streams `*file`, `*symbol`,
   `*span`, `*call_site`, `*imports`, per-language `*_attrs`, etc.
2. `cozo::populate(&store, &graph, Some(&workspace))` — unchanged tail
   (comments, types, inheritance, file_classification, build_meta_files).
3. **NEW** `resolve_and_emit_call_edges(&store, &graph, &mut writer)` —
   walks `*call_site`, resolves to target `*symbol.id`, emits
   `*call_edge{caller_id, callee_id, file_path}`.
4. Indices materialise during `apply_schema` at store open.

### Query, optimised

The rewritten query (see "Final query shape" above) — one rule, three
indexed joins, joined directly against the pre-materialised `*call_edge`
and `*file_classification`. No recursion, no `regex_matches`.

### Bench, per repo size

```
for binary in (baseline, optimised):
  for size in (50, 500, 2000, 5000 files):
    rm -rf <cache>.sqlite
    /usr/bin/time -lp <binary> projects query --path <openclaw>/<subdir> \
      --file test_to_function_map.cozoql > /dev/null
    parse: wall_s, user_s, sys_s, max_rss_mb
    append: <binary>,<size>,<wall_s>,<user_s>,<sys_s>,<max_rss_mb>
```

Each run is cold (cache wiped). Captures build+query end-to-end. A
warm-query-only run (no `rm -rf`) is documented in the README as optional
follow-up.

## Inputs

- **Repos**: subdirectories of openclaw (https://github.com/openclaw/openclaw.git), picked by file count after `find | wc -l`. Target sizes ~50 / 500 / 2000 / 5000 files. Selection captured in the README so re-runs are reproducible.
- **Query file**: the rewritten `test_to_function_map.cozoql`, loaded with `--file`. The orchestrator's `//`-comment strip is replicated in the bench script (Cozo doesn't accept `//`).
- **Binaries**: `target/release/virgil-cli` built once on `master`, once on `feat/test-to-function-map-optimisations`. Bench script handles the checkouts.

## Branch + baseline strategy

Feature branch off master. Bench harness checks out master, builds binary
(`cargo build --release`), runs baseline. Then checks out feat branch,
builds, runs optimised. No git switching mid-bench point — all baseline
runs first, then all optimised runs.

## Error handling

Minimal, per CLAUDE.md guidance.

- `resolve_and_emit_call_edges` bubbles Cozo write errors via
  `anyhow::Result`. No new try/recover.
- Schema bump triggers existing wipe-on-mismatch path in `open_persistent`.
  No new code.
- Bench script: `set -euo pipefail`. Single failure aborts the run.
- Query rewrite: existing cozoql loader surfaces syntax errors.

**Silent-success risk:** if `*call_edge` resolution emits 0 rows
(schema mismatch / missing data), the optimised query returns empty and
the speedup looks artificially huge. Mitigation: bench script greps for a
new `[bench] call_edge_count=N` debug line printed at the end of
`resolve_and_emit_call_edges` and aborts if N is 0 on either binary.

## Testing

- **Schema migration test** — `tests/integration/call_edge.rs`: build a
  tiny fixture workspace, assert `*call_edge` has the expected rows and
  `*call_site` is unchanged.
- **Query equivalence test** — run the OLD query and the NEW query against
  the same fixture store, assert identical output rows (order-insensitive).
  This is the only test that proves the optimisation didn't break the
  answer.
- **`cargo test`** must pass on the feat branch before any bench run, per
  the auto-memory rule on this project.
- Bench script is not unit-tested. Throwaway scaffolding.

## Success criteria

A populated `bench-results.csv` showing, for each (binary, repo size) pair:
- wall_s, user_s, sys_s, max_rss_mb
- a clear speedup column (baseline_wall / optimised_wall) per repo size

Done = CSV exists, equivalence test green, schema test green, `cargo test`
green. Whether the speedup is "good enough" is a separate decision after
the data lands.

## Out of scope

- Concurrent query execution (lever #1 from the earlier discussion).
- Rewriting other queries (`path_traversal`, `xss`, etc.).
- Cozo upstream patches.
- Production rollout, schema migration tooling, telemetry.
- Removing `*call_site` (the raw facts stay; `*call_edge` is additive).
