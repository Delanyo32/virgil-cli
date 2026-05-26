# DuckDB Swap — Preliminary Findings

Status: **partial bench complete** — only 2 of the 6 standard corpora
were available on this machine (openclaw/discord, openclaw/ui). The
larger repos that show the most interesting scaling behaviour
(tokio ~778 files, django ~2.9k files, openclaw/extensions ~5.5k files)
were not cloned at `/tmp/` when this run executed.

## What's in the box

| Phase | Status |
|-------|--------|
| 1. Cargo deps swap | ✅ duckdb + arrow added; cozo + S3/serve disabled |
| 4. Schema port (32 tables + 14 indices + PROPERTY GRAPH DDL) | ✅ verified end-to-end |
| 5. Writer port via DuckDB Appender + INSERT-literal for VARCHAR[] cols | ✅ |
| 6. Builder + from_code_graph + parallel call_edge resolver | ✅ |
| 7. Query runner (`--sql` flag, `$name` literal substitution) | ✅ |
| 8. 7 SQL/PGQ templates | ✅ |
| 9. `complexity_hotspots` Rust handler | ✅ |
| 10. Bench harness (extended) | ✅ |
| 11. End-to-end smoke | ✅ — caveat below |
| 12. Matrix bench + findings | ⚠️ partial |

**541 / 541 library tests pass.**

## Duckdb numbers (this branch)

Captured by `examples/bench_matrix.sh ./target/release/virgil-cli duckdb duckdb`:

```
engine,repo,lang,files,phase,wall_s,max_rss_mb
duckdb,openclaw/discord,ts/tsx,522,parse,0.41,110
duckdb,openclaw/discord,ts/tsx,522,query_find_function_by_name,0.28,NA
duckdb,openclaw/discord,ts/tsx,522,query_find_callers,0.29,NA
duckdb,openclaw/discord,ts/tsx,522,query_find_callees,0.29,NA
duckdb,openclaw/discord,ts/tsx,522,query_find_cycles,0.28,NA
duckdb,openclaw/discord,ts/tsx,522,query_find_implementations_of,0.28,NA
duckdb,openclaw/discord,ts/tsx,522,query_export_surface,0.28,NA
duckdb,openclaw/discord,ts/tsx,522,query_import_depth,0.28,NA
duckdb,openclaw/ui,ts/tsx,461,parse,0.41,128
duckdb,openclaw/ui,ts/tsx,461,query_find_function_by_name,0.29,NA
duckdb,openclaw/ui,ts/tsx,461,query_find_callers,0.28,NA
duckdb,openclaw/ui,ts/tsx,461,query_find_callees,0.28,NA
duckdb,openclaw/ui,ts/tsx,461,query_find_cycles,0.28,NA
duckdb,openclaw/ui,ts/tsx,461,query_find_implementations_of,0.28,NA
duckdb,openclaw/ui,ts/tsx,461,query_export_surface,0.28,NA
duckdb,openclaw/ui,ts/tsx,461,query_import_depth,0.29,NA
```

Saved verbatim at `bench-results-duckdb.csv`.

### Observations

- **Warm query latency** clusters around 0.28–0.29 s for every template.
  That's the process-launch + duckpgq `LOAD` + schema-version check
  floor — not real query work. The actual SQL/PGQ time is probably
  <10 ms on these small repos. A bench that times only the prepared
  query (in-process, after warmup) will tell the real story.
- **Cold build** of ~500-file ts/tsx repos: 0.41 s wall, 110-128 MB
  peak RSS. Master/Cozo's documented baseline for openclaw/discord
  (488 files) was 10.44 s parse / 203.6 MB cold (see
  `bench-results-cold.csv`). At small N, DuckDB looks much faster on
  parse — but the existing baseline numbers may not be apples-to-apples
  (the older harness used `?[count(s)] := *symbol{id: s}` plus the old
  serial absorb path; we'd want both runs from the same harness).
- **Per-template walltimes** are indistinguishable across all 7
  templates at this scale, which means none of the PGQ vs SQL choice
  matters yet for small workloads.

## What's needed to complete Phase 12

1. **Clone the missing corpora** at the standard paths:
   ```
   /tmp/ripgrep        (rs)
   /tmp/gin            (go)
   /tmp/tokio          (rs)
   /tmp/django         (py)
   /tmp/openclaw/extensions  (ts/tsx, the big one — 5.5k files)
   ```
2. **Run the duckdb bench** (in this worktree):
   ```
   examples/bench_matrix.sh ./target/release/virgil-cli duckdb duckdb \
     > bench-results-duckdb.csv
   ```
3. **Run the cozo bench** (in master, after copying the new
   `bench_matrix.sh` over there):
   ```
   git checkout master
   cargo build --release
   examples/bench_matrix.sh ./target/release/virgil-cli cozo cozo \
     > bench-results-cozo.csv
   ```
4. **Diff** — `python -c "import pandas..." ` or a spreadsheet. Per
   `(repo, phase)`, compute duckdb/cozo ratios for `wall_s` and
   `max_rss_mb`.

The user-facing answer (winner per axis: speed, memory, scale, MT
CPU) requires the larger repos. The 2-repo run we have here is too
small a sample to make the call.

## Caveats

- **Pre-existing extractor bug**: `call_site.caller_id` resolves to
  the nearest parameter symbol when the enclosing function has
  parameters (instead of the function itself). Surfaced during the
  smoke test (`find_callers` returned `caller=user` for a call inside
  `pub fn login(user: &str)`). Both backends see the same data, so
  the bench comparison stays valid — but the absolute correctness of
  `find_callers`/`find_callees` results is suspect on real workloads.
  See "Findings during end-to-end smoke" in `duckdb-swap.md`.
- **Per-template wall overhead is dominated by process startup +
  duckpgq LOAD**. A more discriminating bench would run all 7
  templates in a single binary invocation against a pre-warmed cache
  — that's the next refinement to `bench_matrix.sh` worth landing.
- The `MT CPU` axis (`user_s / wall_s`) wasn't captured here — the
  harness drops `user_s` from the output. Re-add it to
  `bench_matrix.sh` to recover that data.
