#!/usr/bin/env bash
# Bench: master vs feat/test-to-function-map-optimisations on openclaw subdirs.
# Outputs bench-results.csv with one row per (binary, subdir) pair.
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

echo "binary,subdir,files,wall_s,user_s,sys_s,max_rss_mb,call_edge_count" > "$RESULTS"

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
  local label="$1" binary="$2" subdir="$3" query_file="$4"
  local target="$OPENCLAW/$subdir"
  local files
  files=$(find "$target" -type f | wc -l | tr -d ' ')

  # Unique project name for this run.
  local proj="bench-$$-$label-$RANDOM"

  # Register the project (cheap — writes metadata only, no cache work).
  "$binary" projects create "$proj" --path "$target" >/dev/null 2>&1

  # Cold start: wipe the cache AFTER registration so the timed query does
  # the full cold build (projects create does not touch the SQLite store).
  rm -rf "$CACHE_DIR"/*.sqlite 2>/dev/null || true

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
    # On macOS the time -lp RSS is in bytes; on linux it's in KB. Detect:
    # values >1e8 are almost certainly bytes (≥100MB).
    if (( rss_kb > 100000000 )); then
      rss_mb=$(awk "BEGIN{printf \"%.1f\", $rss_kb / 1048576}")
    else
      rss_mb=$(awk "BEGIN{printf \"%.1f\", $rss_kb / 1024}")
    fi
  else
    rss_mb="NA"
  fi

  echo "$label,$subdir,$files,$wall,$user,$sys,$rss_mb,$call_edges" >> "$RESULTS"
  echo "[bench] $label $subdir files=$files wall=${wall}s rss=${rss_mb}MB call_edges=$call_edges"
  rm -f "$time_out"
}

build_baseline
build_optimised

for subdir in "${SUBDIRS[@]}"; do
  run_one baseline  "$BASELINE_BIN"  "$subdir" "$REPO_ROOT/examples/test_to_function_map.baseline.cozoql"
done
for subdir in "${SUBDIRS[@]}"; do
  run_one optimised "$OPTIMISED_BIN" "$subdir" "$REPO_ROOT/examples/test_to_function_map.optimised.cozoql"
done

echo
echo "[bench] done. Results in $RESULTS"
column -ts, "$RESULTS"
