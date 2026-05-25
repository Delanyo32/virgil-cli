# Bench: `test_to_function_map` query optimisations

Compares `master` vs `feat/test-to-function-map-optimisations` end-to-end
(cold build + query) over progressively larger openclaw subdirectories.

## Setup

1. Clone openclaw somewhere outside this repo:
   ```bash
   git clone --depth 1 https://github.com/openclaw/openclaw.git /tmp/openclaw
   ```
2. Survey subdir sizes to pick your 4 datapoints:
   ```bash
   find /tmp/openclaw -mindepth 1 -maxdepth 3 -type d \
     -exec sh -c 'printf "%6d  %s\n" "$(find "$1" -type f | wc -l)" "$1"' _ {} \; \
     | sort -n | tail -30
   ```
   Pick four subdirs whose file counts roughly hit 50 / 500 / 2000 / 5000.

## Run

From the repo root, on the feat branch:
```bash
./examples/bench_query_optimisations.sh /tmp/openclaw \
  Source/Foo Source/Bar Source/Baz Source
```

The script:
- Checks out master, builds, saves binary as `target/release/virgil-cli-baseline`
- Returns to your current branch, builds, saves as `target/release/virgil-cli-optimised`
- For each subdir × binary: wipes the project's SQLite cache, runs the
  query end-to-end under `/usr/bin/time -lp`, parses wall/user/sys/RSS,
  greps the resolver's `[bench] call_edge_count=N` line.

Output: `bench-results.csv` at the repo root.

## Phase-separated measurement

By default (`BENCH_PHASES=parse,query`) the script measures each binary in two passes:

1. **parse phase** — cold cache, no-op query (`noop.cozoql`). Wall time captures
   the full parse + populate cost, with essentially zero query work.
2. **query phase** — warm cache (the cache the same binary just built), real query.
   Wall time is the query-only cost.

Each binary builds its own cache and its own warm query runs against that cache —
the baseline never sees an optimised binary's pre-populated `*call_edge` data.

Set `BENCH_PHASES=all` to restore the legacy single-phase cold end-to-end run.

## Reading the results

Columns: `binary, phase, subdir, files, wall_s, user_s, sys_s, max_rss_mb, call_edge_count`.

- **Speedup** = baseline_wall / optimised_wall, per subdir.
- **CPU spread** = user_s / wall_s. >1.0 means the work used multiple
  cores; ≈1.0 means single-threaded.
- **call_edge_count** is your sanity check. `NA` on baseline rows is
  expected — the resolver only runs on the optimised binary. It must be
  >0 on optimised rows; 0 means the bench is invalid.

## Warm-query-only run

To isolate query time from build time:

1. First run normally (cold — populates the cache and gives end-to-end timing):
   ```bash
   ./examples/bench_query_optimisations.sh /tmp/openclaw <subdir1> <subdir2>
   mv bench-results.csv bench-results-cold.csv
   ```
2. Re-run with the wipe disabled (uses the cache built above; ~query-only):
   ```bash
   BENCH_NO_WIPE=1 ./examples/bench_query_optimisations.sh /tmp/openclaw <subdir1> <subdir2>
   mv bench-results.csv bench-results-warm.csv
   ```

The second CSV's `wall_s` column is approximately the query time alone
(plus minimal store-open + script overhead). Compare warm rows across
baseline vs optimised to see the pure query speedup.

Note: between the two runs the script re-checks-out master and rebuilds.
That's incremental cargo work and adds ~5s. The cache built by the cold
run survives because BENCH_NO_WIPE=1 prevents the rm -rf.

## Verify call_edge_count > 0 for every optimised row

```bash
awk -F, 'NR>1 && $1=="optimised" && ($8=="0" || $8=="NA") {print "INVALID:", $0}' bench-results.csv
```

Expected: no output.

## Why these particular changes?

See `docs/superpowers/specs/2026-05-25-test-to-function-map-optimisations-design.md`.
