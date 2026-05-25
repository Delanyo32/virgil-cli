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

## Reading the results

Columns: `binary, subdir, files, wall_s, user_s, sys_s, max_rss_mb, call_edge_count`.

- **Speedup** = baseline_wall / optimised_wall, per subdir.
- **CPU spread** = user_s / wall_s. >1.0 means the work used multiple
  cores; ≈1.0 means single-threaded.
- **call_edge_count** is your sanity check. If it's 0 on either binary,
  the comparison is invalid (likely a schema or extractor mismatch).

## Optional: warm-query-only run

Each scripted run is cold (cache wiped) and includes the build. To
isolate the query speedup from the build cost:

1. Comment out the `rm -rf "$CACHE_DIR"/*.sqlite` line in `run_one`.
2. Run the script twice — second run uses the warm cache.

The second run's wall time is roughly query-only.

## Why these particular changes?

See `docs/superpowers/specs/2026-05-25-test-to-function-map-optimisations-design.md`.
