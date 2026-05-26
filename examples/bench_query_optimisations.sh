#!/usr/bin/env bash
# Bench: master vs feat/test-to-function-map-optimisations on openclaw subdirs.
# Outputs bench-results.csv with one row per (binary, phase, subdir) pair.
#
# Usage:
#   ./examples/bench_query_optimisations.sh <openclaw-clone-path> <subdir1> [<subdir2> ...]
#
# Example:
#   ./examples/bench_query_optimisations.sh /tmp/openclaw \
#       Source/ActorComponent Source/ActorController Source Build
#
# Each <subdir> is benched at its full file count. The script does not
# slice files itself — pick subdirs whose file counts span the range you
# want (~50 / ~500 / ~2000 / ~5000).
#
# Env vars:
#   BENCH_NO_WIPE=1   Skip the per-run cache wipe (warm-query measurement).
#                     Run the script once normally to populate the cache,
#                     then re-run with BENCH_NO_WIPE=1 to time warm queries.
#
#   BENCH_PHASES      Controls which phases are measured (default: parse,query).
#                     parse,query  Run each binary in two passes:
#                                  1. cold-cache with a no-op query (parse phase)
#                                  2. warm-cache with the real query (query phase)
#                                  The warm pass reuses the cache the same binary
#                                  just built — baseline never sees optimised data.
#                     all          Legacy: end-to-end cold run for each real query.

set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <openclaw-clone-path> <subdir1> [<subdir2> ...]" >&2
  exit 1
fi

OPENCLAW="$1"; shift
SUBDIRS=("$@")

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE_DIR="$HOME/Library/Caches/virgil"   # macOS; adjust for linux
BASELINE_BIN="$REPO_ROOT/target/release/virgil-cli-baseline"
OPTIMISED_BIN="$REPO_ROOT/target/release/virgil-cli-optimised"
RESULTS="$REPO_ROOT/bench-results.csv"

echo "binary,phase,subdir,files,wall_s,user_s,sys_s,max_rss_mb,call_edge_count" > "$RESULTS"

build_baseline() {
  echo "[bench] building master binary..."
  git -C "$REPO_ROOT" stash push --include-untracked -m "bench-stash" >/dev/null
  trap 'git -C "$REPO_ROOT" stash pop >/dev/null 2>&1 || true' EXIT
  local cur_branch
  cur_branch=$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD)
  git -C "$REPO_ROOT" checkout master
  (cd "$REPO_ROOT" && cargo build --release)
  cp "$REPO_ROOT/target/release/virgil-cli" "$BASELINE_BIN"
  git -C "$REPO_ROOT" checkout "$cur_branch"
  trap - EXIT
  git -C "$REPO_ROOT" stash pop >/dev/null 2>&1 || true
}

build_optimised() {
  echo "[bench] building feat binary..."
  (cd "$REPO_ROOT" && cargo build --release)
  cp "$REPO_ROOT/target/release/virgil-cli" "$OPTIMISED_BIN"
}

run_one() {
  local label="$1" binary="$2" subdir="$3" query_file="$4" phase="$5"
  local target="$OPENCLAW/$subdir"
  local files
  files=$(find "$target" -type f | wc -l | tr -d ' ')

  # Stable project name so that the warm pass (BENCH_NO_WIPE=1) reuses the
  # sqlite cache built by the preceding cold pass. Projects are deleted after
  # each run so the name doesn't linger in the registry.
  local safe_subdir="${subdir//\//-}"
  local proj="bench-${label}-${safe_subdir}"

  # Register the project (cheap — writes metadata only, no cache work).
  "$binary" projects create "$proj" --path "$target" >/dev/null 2>&1

  # Cold start: wipe the cache AFTER registration so the timed query does
  # the full cold build (projects create does not touch the SQLite store).
  # Set BENCH_NO_WIPE=1 to skip this (warm-query mode).
  if [[ "${BENCH_NO_WIPE:-0}" != "1" ]]; then
    rm -rf "$CACHE_DIR"/*.sqlite 2>/dev/null || true
  fi

  # Capture full stderr (time -lp is on stderr; resolver's call_edge_count too).
  local time_out
  time_out=$(mktemp)
  # `--cozoscript` would require shell escaping; --file is cleaner.
  /usr/bin/time -lp "$binary" projects query "$proj" \
    --file "$query_file" >/dev/null 2>"$time_out" || true

  # Clean up the registry entry.
  "$binary" projects delete "$proj" >/dev/null 2>&1 || true

  local wall user sys rss_kb call_edges
  wall=$(awk '/^real / {print $2}' "$time_out")
  user=$(awk '/^user / {print $2}' "$time_out")
  sys=$(awk '/^sys / {print $2}' "$time_out")
  rss_kb=$(awk '/maximum resident set size/ {print $1}' "$time_out")
  call_edges=$(awk '/^\[bench\] call_edge_count=/ {gsub("[^0-9]","",$0); print}' "$time_out")
  call_edges="${call_edges:-NA}"

  local rss_mb
  if [[ -n "${rss_kb:-}" ]]; then
    # On macOS, /usr/bin/time -lp reports RSS in bytes; on Linux it's in KB.
    case "$(uname -s)" in
      Darwin) rss_mb=$(awk "BEGIN{printf \"%.1f\", $rss_kb / 1048576}") ;;
      Linux)  rss_mb=$(awk "BEGIN{printf \"%.1f\", $rss_kb / 1024}") ;;
      *)      rss_mb="NA" ;;
    esac
  else
    rss_mb="NA"
  fi

  echo "$label,$phase,$subdir,$files,$wall,$user,$sys,$rss_mb,$call_edges" >> "$RESULTS"
  echo "[bench] $label $phase $subdir files=$files wall=${wall}s rss=${rss_mb}MB call_edges=$call_edges"
  rm -f "$time_out"
}

build_baseline
build_optimised

PHASES="${BENCH_PHASES:-parse,query}"

for subdir in "${SUBDIRS[@]}"; do
  case "$PHASES" in
    "parse,query")
      # Baseline: parse (cold, noop), then query (warm, real).
      run_one baseline "$BASELINE_BIN" "$subdir" "$REPO_ROOT/examples/noop.cozoql" parse
      BENCH_NO_WIPE=1 run_one baseline "$BASELINE_BIN" "$subdir" "$REPO_ROOT/examples/test_to_function_map.baseline.cozoql" query
      # Optimised: same pattern, fresh cache.
      run_one optimised "$OPTIMISED_BIN" "$subdir" "$REPO_ROOT/examples/noop.cozoql" parse
      BENCH_NO_WIPE=1 run_one optimised "$OPTIMISED_BIN" "$subdir" "$REPO_ROOT/examples/test_to_function_map.optimised.cozoql" query
      ;;
    all)
      # Legacy: end-to-end cold for the real query.
      run_one baseline  "$BASELINE_BIN"  "$subdir" "$REPO_ROOT/examples/test_to_function_map.baseline.cozoql"  all
      run_one optimised "$OPTIMISED_BIN" "$subdir" "$REPO_ROOT/examples/test_to_function_map.optimised.cozoql" all
      ;;
    *)
      echo "[bench] unknown BENCH_PHASES=$PHASES (use parse,query or all)" >&2
      exit 1
      ;;
  esac
done

echo
echo "[bench] done. Results in $RESULTS"
column -ts, "$RESULTS"
